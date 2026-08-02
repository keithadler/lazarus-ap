//! Three CRTs, five computers: the real cockpit arrangement.
//!
//! The orbiter had three Display Electronics Units — three keyboards
//! and three CRTs — and any of them could be assigned to any GPC with
//! the GPC/CRT key. This test runs that: three DEUs on three display
//! buses, each polled by a different computer, with a fourth machine
//! listening to all of them (the BFS arrangement).

use lazarus_ap::deu::{key, Deu};
use lazarus_ap::gpc::{Gpc, RedundantSet};
use lazarus_ap::Memory;

const KEYBUF: u32 = 0x1800;
const DEU_IUA: u32 = 0x0C;

/// A GPC whose BCE `bus` polls the DEU on that bus into KEYBUF.
fn station_gpc(bus: usize, listen_only: bool) -> Gpc {
    let mut mem = Memory::new(0x4000);
    let poll: u32 = (DEU_IUA << 19) | (1 << 16) | 1;
    mem.load_halfwords(
        0x200,
        &[
            0xF200,
            KEYBUF as u16,
            0xB000 | 200,
            0xC000,
            0b11110001_00000000,
            0,
            (poll >> 16) as u16,
            poll as u16,
            0xF000,
            0x0204,
        ],
    )
    .unwrap();
    // listener: receive the keystroke words without commanding
    mem.load_halfwords(
        0x240,
        &[
            0xF200,
            KEYBUF as u16,
            0xB000 | 400,
            0b011_00000_00000000, // #RDS: one word into KEYBUF
            0xE800,               // #SIB
            0xC000,               // #DLYI pad: the #BU below is a long
            0xF000,               // instruction and must sit on an even
            0x0243,               // halfword boundary

        ],
    )
    .unwrap();
    mem.write_h(KEYBUF, 0xFF).unwrap();
    let mut gpc = Gpc::new(mem);
    gpc.cpu.psw.wait = true;
    {
        let mut iop = gpc.iop.borrow_mut();
        iop.halted = false;
        iop.bces[bus].busy = true;
        iop.bces[bus].pc = if listen_only { 0x240 } else { 0x200 };
        iop.mia_rcvr_enable = 0xFFFF_0000;
        if listen_only {
            iop.bces[bus].iuar = DEU_IUA as u8;
        } else {
            iop.mia_xmtr_enable = 1 << (31 - bus as u32);
        }
    }
    gpc
}

#[test]
fn three_crts_three_computers_and_a_shadow() {
    // CRT 1 on bus 4 driven by GPC 0, CRT 2 on bus 5 by GPC 1,
    // CRT 3 on bus 6 by GPC 2 — plus GPC 3 listening on bus 4.
    let mut set = RedundantSet::new(vec![
        station_gpc(4, false),
        station_gpc(5, false),
        station_gpc(6, false),
        station_gpc(4, true),
    ]);
    for (bus, keys) in [
        (4usize, vec![key::OPS, 2, 0, 1, key::PRO]),
        (5, vec![key::SPEC, 5, 1, key::PRO]),
        (6, vec![key::ITEM, 3, 6, key::EXEC]),
    ] {
        let mut deu = Deu::new(bus, DEU_IUA as u8, 4, 16);
        deu.type_keys(&keys);
        set.subsystems.push(Box::new(deu));
    }
    // The poll loops keep running, so collect what each computer hears
    // as it arrives rather than reading once at the end.
    let mut heard: Vec<Vec<u16>> = vec![Vec::new(); 4];
    for _ in 0..2000 {
        set.step();
        for g in 0..4 {
            let k = set.gpcs[g].cpu.mem.read_h(KEYBUF).unwrap();
            if k != 0xFF && heard[g].last() != Some(&k) {
                heard[g].push(k);
            }
            let _ = set.gpcs[g].cpu.mem.write_h(KEYBUF, 0xFF);
        }
    }

    // Each computer received only its own station's keystrokes.
    assert_eq!(
        heard[0],
        vec![key::OPS as u16, 2, 0, 1, key::PRO as u16],
        "GPC 0 heard CRT 1: OPS 201 PRO"
    );
    assert_eq!(
        heard[1],
        vec![key::SPEC as u16, 5, 1, key::PRO as u16],
        "GPC 1 heard CRT 2: SPEC 51 PRO"
    );
    assert_eq!(
        heard[2],
        vec![key::ITEM as u16, 3, 6, key::EXEC as u16],
        "GPC 2 heard CRT 3: ITEM 36 EXEC"
    );
    // The shadow on bus 4 overheard CRT 1's traffic without commanding.
    assert_eq!(heard[3], heard[0], "shadow overheard CRT 1 exactly");
    assert_eq!(
        set.gpcs[3].iop.borrow().mia_xmtr_enable,
        0,
        "shadow never transmits"
    );
    for i in 0..4 {
        assert!(set.dead[i].is_none(), "GPC {i} alive");
    }
}
