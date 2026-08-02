//! Browser interface: the whole emulator, compiled to WebAssembly.
//!
//! Deliberately dependency-free — no wasm-bindgen, just `extern "C"`
//! entry points over a single global machine, with JavaScript reading
//! results out of linear memory. That keeps the build reproducible from
//! a bare toolchain (`cargo build --target wasm32-unknown-unknown
//! --release`) and the artifact small enough to embed in a page.
//!
//! Two machines are exposed: a single GPC running a linked flight image
//! (the walkthrough console) and the five-computer redundant set.

use crate::deu::Deu;
use crate::gpc::{ForceVotedActuator, Gpc, RedundantSet};
use crate::halucp::HalUcp;
use crate::{decode, fcm, Cpu, Memory};

const MBOX: u32 = 0x1000;

struct Single {
    cpu: Cpu,
    ucp: HalUcp,
    done: bool,
}

static mut SINGLE: Option<Single> = None;
static mut SET: Option<RedundantSet> = None;
static mut BUF: Vec<u8> = Vec::new();

fn single() -> &'static mut Single {
    unsafe { (*core::ptr::addr_of_mut!(SINGLE)).as_mut().expect("boot first") }
}

/// Scratch buffer JavaScript writes into (image bytes, symbols JSON) and
/// reads out of (output text, register dumps).
#[no_mangle]
pub extern "C" fn buf_reserve(len: usize) -> *mut u8 {
    unsafe {
        let b = &mut *core::ptr::addr_of_mut!(BUF);
        b.clear();
        b.resize(len, 0);
        b.as_mut_ptr()
    }
}

#[no_mangle]
pub extern "C" fn buf_len() -> usize {
    unsafe { (*core::ptr::addr_of!(BUF)).len() }
}

#[no_mangle]
pub extern "C" fn buf_ptr() -> *const u8 {
    unsafe { (*core::ptr::addr_of!(BUF)).as_ptr() }
}

fn buf_take() -> Vec<u8> {
    unsafe { core::mem::take(&mut *core::ptr::addr_of_mut!(BUF)) }
}

fn buf_put(s: &str) {
    unsafe {
        let b = &mut *core::ptr::addr_of_mut!(BUF);
        b.clear();
        b.extend_from_slice(s.as_bytes());
    }
}

/// Boot a flight image previously written into the scratch buffer.
/// `ioinit`/`intrap`/`iocode`/`iobuf` come from the linker's symbol
/// table; pass 0 for `ioinit` to run without runtime-I/O traps.
#[no_mangle]
pub extern "C" fn boot(entry: u32, ioinit: u32, intrap: u32, iocode: u32, iobuf: u32) {
    let image = buf_take();
    let mut cpu = Cpu::new(Memory::full());
    let json = alloc::format!("{{\"entryPoint\": {entry}}}");
    fcm::boot(&mut cpu, &image, Some(&json)).ok();
    let ucp = if ioinit == 0 {
        HalUcp::new(u32::MAX >> 1, 0, 0, 0)
    } else {
        HalUcp::new(ioinit, intrap, iocode, iobuf)
    };
    unsafe { SINGLE = Some(Single { cpu, ucp, done: false }) }
}

/// Advance up to `n` instructions; returns 1 when the program has ended.
#[no_mangle]
pub extern "C" fn step(n: u32) -> u32 {
    let s = single();
    for _ in 0..n {
        if s.done {
            break;
        }
        let nia = s.cpu.expand_branch(s.cpu.psw.ic);
        s.ucp.check_trap(&mut s.cpu, nia);
        match s.cpu.step() {
            Ok(_) => {}
            Err(crate::Trap::UninitializedInterrupt { code, .. }) => {
                let ea = ((s.cpu.psw.ea_high as u32) << 15) | (code as u32 & 0x7FFF);
                if s.cpu.mem.read_h(ea).unwrap_or(0) == crate::halucp::SVC_END {
                    s.ucp.flush();
                    s.done = true;
                }
            }
            Err(_) => s.done = true,
        }
        if s.cpu.psw.wait {
            s.done = true;
        }
    }
    s.done as u32
}

/// Machine state for the console: address, mnemonic, registers, CC.
/// Written into the scratch buffer as JSON.
#[no_mangle]
pub extern "C" fn state() {
    let s = single();
    let nia = s.cpu.expand_branch(s.cpu.psw.ic);
    let hw1 = s.cpu.mem.read_h(nia).unwrap_or(0);
    let hw2 = s.cpu.mem.read_h(nia.wrapping_add(1)).unwrap_or(0);
    let m = decode::decode(hw1, hw2)
        .map(|d| d.instr.mnemonic())
        .unwrap_or("???");
    let regs: alloc::vec::Vec<alloc::string::String> = (0..8)
        .map(|i| alloc::format!("\"{:08X}\"", s.cpu.r(i)))
        .collect();
    buf_put(&alloc::format!(
        "{{\"ic\":{},\"m\":\"{}\",\"hw\":{},\"r\":[{}],\"cc\":{},\"done\":{}}}",
        nia,
        m,
        hw1,
        regs.join(","),
        s.cpu.psw.cc,
        s.done
    ));
}

/// The program's accumulated output text.
#[no_mangle]
pub extern "C" fn output() {
    let s = single();
    let text = s.ucp.output.clone();
    buf_put(&text);
}

/// Read one halfword of main storage (for inspecting results).
#[no_mangle]
pub extern "C" fn peek(addr: u32) -> u32 {
    single().cpu.mem.read_h(addr).unwrap_or(0) as u32
}

// ---- the redundant set ----

/// Build the five-computer set; `fault` is the GPC given a corrupted
/// sensor (5 = none), `kill` a GPC to disable (5 = none).
#[no_mangle]
pub extern "C" fn set_new(fault: u32, kill: u32) {
    let mut gpcs: alloc::vec::Vec<Gpc> = (0..4)
        .map(|id| crate::demo::build_gpc(id, id as u32 == fault))
        .collect();
    let bfs = crate::demo::build_gpc(4, false);
    bfs.iop.borrow_mut().mia_xmtr_enable = 0;
    gpcs.push(bfs);
    if kill < 5 {
        gpcs[kill as usize]
            .cpu
            .mem
            .write_h(0x100, 0b00110_001_11100_010)
            .ok();
    }
    let mut set = RedundantSet::new(gpcs);
    set.actuators.push(ForceVotedActuator::new(8, 5));
    // A DEU on the display bus so the crew station has something to talk to.
    set.subsystems.push(alloc::boxed::Box::new(Deu::new(4, 0x0C, 6, 26)));
    unsafe { SET = Some(set) }
}

fn set_ref() -> &'static mut RedundantSet {
    unsafe { (*core::ptr::addr_of_mut!(SET)).as_mut().expect("set_new first") }
}

#[no_mangle]
pub extern "C" fn set_step(n: u32) {
    let set = set_ref();
    for _ in 0..n {
        set.step();
    }
}

/// Per-GPC mailboxes, verdict and liveness, plus the actuator, as JSON.
#[no_mangle]
pub extern "C" fn set_state(verdict_addr: u32) {
    let set = set_ref();
    let mut out = alloc::string::String::from("{\"gpcs\":[");
    for i in 0..set.gpcs.len() {
        if i > 0 {
            out.push(',');
        }
        let g = &set.gpcs[i];
        let boxes: alloc::vec::Vec<alloc::string::String> = (0..4)
            .map(|j| alloc::format!("{}", g.cpu.mem.read_h(MBOX + j).unwrap_or(0)))
            .collect();
        out.push_str(&alloc::format!(
            "{{\"m\":[{}],\"v\":{},\"dead\":{}}}",
            boxes.join(","),
            g.cpu.mem.read_h(verdict_addr).unwrap_or(0),
            set.dead[i].is_some()
        ));
    }
    let act = set.actuators[0].output().unwrap_or(-1);
    let byp: alloc::vec::Vec<alloc::string::String> = (0..4)
        .filter(|&p| set.actuators[0].bypassed[p])
        .map(|p| alloc::format!("{p}"))
        .collect();
    out.push_str(&alloc::format!(
        "],\"act\":{},\"byp\":[{}]}}",
        act,
        byp.join(",")
    ));
    buf_put(&out);
}

// ---- crew stations: three CRTs, three computers ----

static mut CREW: Option<RedundantSet> = None;
/// Keystrokes each computer has received. The poll loop overwrites the
/// buffer with the empty marker within a few slices, so the arrival has
/// to be recorded as it happens rather than sampled by the browser.
static mut HEARD: Option<alloc::vec::Vec<alloc::vec::Vec<u8>>> = None;

fn crew_ref() -> &'static mut RedundantSet {
    unsafe { (*core::ptr::addr_of_mut!(CREW)).as_mut().expect("crew_new first") }
}

/// Three DEUs on display buses 4/5/6, each polled by its own GPC, plus a
/// fourth machine listening to CRT 1 (the BFS arrangement).
#[no_mangle]
pub extern "C" fn crew_new() {
    const KEYBUF: u32 = 0x1800;
    let station = |bus: usize, listen: bool| -> Gpc {
        let mut mem = Memory::new(0x4000);
        let poll: u32 = (0x0C << 19) | (1 << 16) | 1;
        mem.load_halfwords(
            0x200,
            &[0xF200, KEYBUF as u16, 0xB000 | 200, 0xC000,
              0b11110001_00000000, 0, (poll >> 16) as u16, poll as u16,
              0xF000, 0x0204],
        ).ok();
        mem.load_halfwords(
            0x240,
            &[0xF200, KEYBUF as u16, 0xB000 | 400, 0b011_00000_00000000,
              0xE800, 0xC000, 0xF000, 0x0243],
        ).ok();
        mem.write_h(KEYBUF, 0xFF).ok();
        let mut g = Gpc::new(mem);
        g.cpu.psw.wait = true;
        {
            let mut iop = g.iop.borrow_mut();
            iop.halted = false;
            iop.bces[bus].busy = true;
            iop.bces[bus].pc = if listen { 0x240 } else { 0x200 };
            iop.mia_rcvr_enable = 0xFFFF_0000;
            if listen {
                iop.bces[bus].iuar = 0x0C;
            } else {
                iop.mia_xmtr_enable = 1 << (31 - bus as u32);
            }
        }
        g
    };
    let mut set = RedundantSet::new(alloc::vec![
        station(4, false), station(5, false), station(6, false), station(4, true)
    ]);
    for bus in [4usize, 5, 6] {
        set.subsystems.push(alloc::boxed::Box::new(Deu::new(bus, 0x0C, 5, 22)));
    }
    unsafe {
        CREW = Some(set);
        HEARD = Some(alloc::vec![alloc::vec::Vec::new(); 4]);
    }
}

/// Press a key at station `n` (0-2).
#[no_mangle]
pub extern "C" fn crew_key(n: u32, code: u32) {
    let set = crew_ref();
    if let Some(d) = set.subsystems[n as usize].as_any_mut().downcast_mut::<Deu>() {
        d.press(code as u8);
    }
}

#[no_mangle]
pub extern "C" fn crew_step(n: u32) {
    const KEYBUF: u32 = 0x1800;
    let set = crew_ref();
    let heard = unsafe { (*core::ptr::addr_of_mut!(HEARD)).as_mut().unwrap() };
    for _ in 0..n {
        set.step();
        for g in 0..set.gpcs.len().min(4) {
            let k = set.gpcs[g].cpu.mem.read_h(KEYBUF).unwrap_or(0xFF);
            if k != 0xFF {
                if heard[g].last() != Some(&(k as u8)) {
                    heard[g].push(k as u8);
                }
                set.gpcs[g].cpu.mem.write_h(KEYBUF, 0xFF).ok();
            }
        }
    }
}

/// What each computer has heard, and each CRT shows, as JSON.
#[no_mangle]
pub extern "C" fn crew_state() {
    const KEYBUF: u32 = 0x1800;
    let set = crew_ref();
    let _ = KEYBUF;
    let heard = unsafe { (*core::ptr::addr_of!(HEARD)).as_ref().unwrap() };
    let mut out = alloc::string::String::from("{\"gpcs\":[");
    for i in 0..set.gpcs.len().min(4) {
        if i > 0 { out.push(','); }
        let list: alloc::vec::Vec<alloc::string::String> =
            heard[i].iter().map(|k| alloc::format!("{k}")).collect();
        out.push_str(&alloc::format!("[{}]", list.join(",")));
    }
    out.push_str("],\"crts\":[");
    for n in 0..3usize {
        if n > 0 { out.push(','); }
        let d = set.subsystems[n].as_any().downcast_ref::<Deu>().unwrap();
        out.push('"');
        out.push_str(&d.screen_text().join("\\n"));
        out.push('"');
    }
    out.push_str("]}");
    buf_put(&out);
}

extern crate alloc;
