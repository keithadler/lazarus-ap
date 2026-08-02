//! Phase 5: the crew interface. A GPC polls a DEU keyboard and paints
//! its CRT — as real bus transactions driven by BCE programs (#MIN to
//! poll keystrokes, #MOUT to write the display), with the DEU sitting
//! on a display bus as a subsystem. A BFS-style listener GPC on the
//! same bus overhears every keystroke — exactly how the real BFS
//! tracked what the crew typed at PASS.

use lazarus_ap::deu::{key, Deu};
use lazarus_ap::gpc::{Gpc, RedundantSet};
use lazarus_ap::Memory;

const DISPLAY_BUS: usize = 4;
const DEU_IUA: u32 = 0x0C;
const KEYBUF: u32 = 0x1800; // received keystrokes (base)
const TEXT: u32 = 0x1810; // display text to send (base + 0x10)

/// BCE program on the display bus: poll 8 keystrokes into KEYBUF
/// (#MIN), then write 7 characters from TEXT to the CRT (#MOUT).
fn crew_bce_program(mem: &mut Memory) {
    let poll_cmd: u32 = (DEU_IUA << 19) | (1 << 16) | 8; // OP_POLL_KEYS
    let disp_cmd: u32 = (DEU_IUA << 19) | (2 << 16) | 7; // OP_DISPLAY_WRITE, cell 0, 7 chars
    mem.load_halfwords(
        0x200,
        &[
            0xF200,
            KEYBUF as u16, // #LBR KEYBUF
            0xB000 | 100,  // #LTOI 100
            0xC000,        // #DLYI (pad to even for #MIN)
            0b11110001_00000000,
            7, // #MIN disp 0, transfer count 7 (= 8 words)
            (poll_cmd >> 16) as u16,
            poll_cmd as u16, // its command fullword
            0b11110101_00010000,
            6, // #MOUT disp 0x10, count 6 (= 7 words)
            (disp_cmd >> 16) as u16,
            disp_cmd as u16,
            0xE800, // #SIB
            0x0800, // #WAT
        ],
    )
    .unwrap();
    for (i, ch) in b"OPS 201".iter().enumerate() {
        mem.write_h(TEXT + i as u32, *ch as u16).unwrap();
    }
}

fn crew_gpc(listener_only: bool) -> Gpc {
    let mut mem = Memory::new(0x4000);
    crew_bce_program(&mut mem);
    // Eavesdropper program (BFS arrangement): receive the 8 keystroke
    // words the DEU sends the commander, from the same bus.
    mem.load_halfwords(
        0x240,
        &[
            0xF200,
            KEYBUF as u16, // #LBR KEYBUF
            0xB000 | 200,  // #LTOI 200
            0b011_00111_00000000, // #RDS count 7 (8 words), disp 0
            0xE800,        // #SIB
            0x0800,        // #WAT
        ],
    )
    .unwrap();
    let gpc = Gpc::new(mem);
    let mut iop = gpc.iop.borrow_mut();
    iop.halted = false;
    iop.bces[DISPLAY_BUS].busy = true;
    iop.bces[DISPLAY_BUS].pc = if listener_only { 0x240 } else { 0x200 };
    iop.mia_rcvr_enable = 1 << (31 - DISPLAY_BUS as u32);
    if !listener_only {
        iop.mia_xmtr_enable = 1 << (31 - DISPLAY_BUS as u32);
    } else {
        // a listen-mode BCE's IUAR would come from a listen command;
        // configured directly here (the DEU's address)
        iop.bces[DISPLAY_BUS].iuar = DEU_IUA as u8;
    }
    drop(iop);
    gpc
}

#[test]
fn keyboard_poll_and_display_write() {
    let mut commander = crew_gpc(false);
    commander.cpu.psw.wait = true;
    let mut shadow = crew_gpc(true); // BFS-style eavesdropper
    shadow.cpu.psw.wait = true;
    let mut set = RedundantSet::new(vec![commander, shadow]);
    let mut deu = Deu::new(DISPLAY_BUS, DEU_IUA as u8, 4, 16);
    deu.type_keys(&[key::OPS, 2, 0, 1, key::PRO]);
    set.subsystems.push(Box::new(deu));
    set.run(40);

    assert!(set.dead[0].is_none());
    let gpc = &set.gpcs[0];
    // the keystrokes crossed the display bus into GPC memory
    let keys: Vec<u16> = (0..8)
        .map(|i| gpc.cpu.mem.read_h(KEYBUF + i).unwrap())
        .collect();
    assert_eq!(
        keys,
        vec![
            key::OPS as u16,
            2,
            0,
            1,
            key::PRO as u16,
            0xFF,
            0xFF,
            0xFF
        ],
        "OPS 2 0 1 PRO arrived, then empty markers"
    );
    let iop = gpc.iop.borrow();
    assert!(iop.bces[DISPLAY_BUS].indicator, "BCE program completed");
    assert!(!iop.bces[DISPLAY_BUS].error);
    drop(iop);
    // and the GPC painted the CRT
    let deu = set.subsystems[0]
        .as_any()
        .downcast_ref::<Deu>()
        .unwrap();
    assert_eq!(deu.screen_text()[0], "OPS 201         ");
    // the shadow GPC overheard every keystroke on the same bus — how
    // the BFS tracked what the crew typed at PASS
    let shadow = &set.gpcs[1];
    let heard: Vec<u16> = (0..8)
        .map(|i| shadow.cpu.mem.read_h(KEYBUF + i).unwrap())
        .collect();
    assert_eq!(heard, keys, "eavesdropper saw the same keystrokes");
    let iop = shadow.iop.borrow();
    assert!(iop.bces[DISPLAY_BUS].indicator && !iop.bces[DISPLAY_BUS].error);
}
