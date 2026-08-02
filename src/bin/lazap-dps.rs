//! Interactive DPS crew station: type at the emulated DEU keyboard,
//! watch the CRT, and see what the GPC hears over the display bus.
//!
//! Everything between your keystroke and the "GPC HEARD" line is the
//! emulated stack: keys queue in the DEU; the GPC's BCE program polls
//! them across the display bus with real #MIN transactions into main
//! storage; the CRT header was painted by the GPC with a #MOUT.
//!
//! Keys: 0-9 a-f digits · +,-,. · o=OPS s=SPEC i=ITEM x=EXEC p=PRO
//! r=RESUME l=CLEAR u=SYS SUMM t=FAULT SUMM g=GPC/CRT z=I/O RESET
//! k=ACK · q or Ctrl-C quits.
//!
//! `--demo` types "OPS 2 0 1 PRO" itself and exits (non-interactive
//! smoke test).

use lazarus_ap::deu::{key, Deu};
use lazarus_ap::gpc::{Gpc, RedundantSet};
use lazarus_ap::Memory;
use std::io::{Read, Write};
use std::sync::mpsc;

const BUS: usize = 4;
const IUA: u32 = 0x0C;
const KEYBUF: u32 = 0x1800;
const TEXT: u32 = 0x1810;

fn build() -> RedundantSet {
    let mut mem = Memory::new(0x4000);
    let poll_cmd: u32 = (IUA << 19) | (1 << 16) | 1; // poll 1 keystroke
    let header = b"LAZARUS AP-101S"; // 15 chars: fits the count nibble
    let disp_cmd: u32 = (IUA << 19) | (2 << 16) | header.len() as u32;
    // BCE program: paint the header once (#MOUT), then poll keystrokes
    // forever (#MIN loop).
    mem.load_halfwords(
        0x200,
        &[
            0xF200,
            KEYBUF as u16, // #LBR
            0xB000 | 100,  // #LTOI 100
            0xC000,        // #DLYI (even alignment)
            0b11110101_00010000,
            (header.len() - 1) as u16, // #MOUT disp 0x10, count n-1
            (disp_cmd >> 16) as u16,
            disp_cmd as u16,
            // poll loop:
            0b11110001_00000000,
            0, // #MIN disp 0, 1 word
            (poll_cmd >> 16) as u16,
            poll_cmd as u16,
            0xF000, 0x0208, // #BU 0x208
        ],
    )
    .unwrap();
    for (i, ch) in header.iter().enumerate() {
        mem.write_h(TEXT + i as u32, *ch as u16).unwrap();
    }
    // 0xFF = "no keystroke": the digit 0's key code is 0, so the
    // buffer's idle value must be the DEU's empty marker.
    mem.write_h(KEYBUF, 0xFF).unwrap();
    let mut gpc = Gpc::new(mem);
    gpc.cpu.psw.wait = true; // bus-side demo; CPU software comes later
    {
        let mut iop = gpc.iop.borrow_mut();
        iop.halted = false;
        iop.bces[BUS].busy = true;
        iop.bces[BUS].pc = 0x200;
        iop.mia_xmtr_enable = 1 << (31 - BUS as u32);
        iop.mia_rcvr_enable = 1 << (31 - BUS as u32);
    }
    let mut set = RedundantSet::new(vec![gpc]);
    set.subsystems.push(Box::new(Deu::new(BUS, IUA as u8, 6, 26)));
    set
}

fn key_for(c: u8) -> Option<u8> {
    Some(match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'+' => key::PLUS,
        b'-' => key::MINUS,
        b'.' => key::DOT,
        b'o' => key::OPS,
        b's' => key::SPEC,
        b'i' => key::ITEM,
        b'x' => key::EXEC,
        b'p' => key::PRO,
        b'r' => key::RESUME,
        b'l' => key::CLEAR,
        b'u' => key::SYS_SUMM,
        b't' => key::FAULT_SUMM,
        b'g' => key::GPC_CRT,
        b'z' => key::IO_RESET,
        b'k' => key::ACK,
        _ => return None,
    })
}

fn key_name(k: u16) -> String {
    match k as u8 {
        0..=15 => format!("{:X}", k),
        key::PLUS => "+".into(),
        key::MINUS => "-".into(),
        key::DOT => ".".into(),
        key::OPS => "OPS".into(),
        key::SPEC => "SPEC".into(),
        key::ITEM => "ITEM".into(),
        key::EXEC => "EXEC".into(),
        key::PRO => "PRO".into(),
        key::RESUME => "RESUME".into(),
        key::CLEAR => "CLR".into(),
        key::SYS_SUMM => "SYS".into(),
        key::FAULT_SUMM => "FAULT".into(),
        key::GPC_CRT => "GPC".into(),
        key::IO_RESET => "I/O".into(),
        key::ACK => "ACK".into(),
        _ => "?".into(),
    }
}

fn draw(set: &RedundantSet, heard: &[u16], interactive: bool) {
    let deu = set.subsystems[0].as_any().downcast_ref::<Deu>().unwrap();
    let mut out = String::new();
    if interactive {
        out.push_str("\x1b[2J\x1b[H");
    }
    out.push_str("+--------------------------+  AP-101S DPS CREW STATION\r\n");
    for line in deu.screen_text() {
        out.push_str(&format!("|{line}|\r\n"));
    }
    out.push_str("+--------------------------+\r\n");
    let log: Vec<String> = heard.iter().map(|&k| key_name(k)).collect();
    out.push_str(&format!("GPC HEARD (via display bus): {}\r\n", log.join(" ")));
    if interactive {
        out.push_str("keys: 0-9 a-f + - . o=OPS s=SPEC i=ITEM x=EXEC p=PRO l=CLR q=quit\r\n");
    }
    print!("{out}");
    std::io::stdout().flush().ok();
}

fn main() {
    let demo = std::env::args().any(|a| a == "--demo");
    let mut set = build();
    let mut heard: Vec<u16> = Vec::new();

    let pump = |set: &mut RedundantSet, heard: &mut Vec<u16>| {
        for _ in 0..60 {
            set.step();
            let k = set.gpcs[0].cpu.mem.read_h(KEYBUF).unwrap();
            if k != 0xFF {
                heard.push(k);
                set.gpcs[0].cpu.mem.write_h(KEYBUF, 0xFF).unwrap();
            }
        }
    };

    if demo {
        pump(&mut set, &mut heard); // header paint
        for k in [key::OPS, 2, 0, 1, key::PRO] {
            set.subsystems[0]
                .as_any_mut()
                .downcast_mut::<Deu>()
                .unwrap()
                .press(k);
            pump(&mut set, &mut heard);
        }
        draw(&set, &heard, false);
        return;
    }

    // interactive: raw terminal, reader thread
    std::process::Command::new("stty")
        .args(["raw", "-echo"])
        .stdin(std::process::Stdio::inherit())
        .status()
        .ok();
    let (tx, rx) = mpsc::channel::<u8>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 1];
        while std::io::stdin().read_exact(&mut buf).is_ok() {
            if tx.send(buf[0]).is_err() {
                break;
            }
        }
    });
    loop {
        while let Ok(c) = rx.try_recv() {
            if c == b'q' || c == 3 {
                std::process::Command::new("stty")
                    .args(["sane"])
                    .stdin(std::process::Stdio::inherit())
                    .status()
                    .ok();
                println!();
                return;
            }
            if let Some(k) = key_for(c.to_ascii_lowercase()) {
                set.subsystems[0]
                    .as_any_mut()
                    .downcast_mut::<Deu>()
                    .unwrap()
                    .press(k);
            }
        }
        pump(&mut set, &mut heard);
        if heard.len() > 16 {
            let n = heard.len() - 16;
            heard.drain(0..n);
        }
        draw(&set, &heard, true);
        std::thread::sleep(std::time::Duration::from_millis(80));
    }
}
