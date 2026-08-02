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

/// The full four-processor echo pipeline, all in GPC software:
/// crew keystroke -> DEU -> [BCE A #MIN poll] -> main storage ->
/// [CPU: glyph lookup, cursor bump, then a real PC LOAD LOCAL STORE
/// into MSC register C6] -> [MSC @SEC external call -> @SIO] ->
/// [BCE B #MOUT] -> DEU CRT. The C6 mailbox mechanism is App. II
/// p. II-62; the LS command word is App. I p. I-27.
#[test]
fn cpu_tasked_keyboard_echo() {
    use lazarus_ap::asm::assemble;
    let mut mem = Memory::new(0x4000);

    // CPU: watch KEYBUF; on a keystroke, place its glyph, advance the
    // display cell in BCE B's #MOUT command word, and task the MSC.
    let cpu_src = "
        ORG  0x100
        LA   2,GLYPHS       ; glyph table base (GR2)
LOOP:   LH   5,0x1800       ; KEYBUF
        CHI  5,0x00FF
        BC   4,LOOP         ; 0xFF = no key yet
        LH   6,0(5,2)       ; glyph for key code
        STH  6,0x1810       ; TEXT
        LH   4,FFCON        ; consume the keystroke
        STH  4,0x1800
        L    7,0x1A4        ; bump the CRT cell in BCE B's #MOUT command
        A    7,SIXTEEN
        ST   7,0x1A4
        L    2,LSCW         ; task the MSC: C6 <- echo program (PCO
        L    1,PGMADDR      ; LOAD LOCAL STORE, MSC bank C word 6)
        DC   H(0xD9EA)      ; PC 1,2
        LA   2,GLYPHS       ; restore glyph base
        B    LOOP
FFCON:  DC   H(0x00FF)
SIXTEEN: DC  F(16)
LSCW:   DC   F(0xA00B0000)
PGMADDR: DC  F(0x160)
        ORG  0x1900
GLYPHS: DC   H(0x30)
";
    // glyph table: '0'-'9','A'-'F', then one letter per function key
    let mut src = cpu_src.trim_end().to_string();
    for ch in b"123456789ABCDEF+-.OSIXPRLUTGZK" {
        src.push_str(&format!("\n        DC H({})", ch));
    }
    let prog = assemble(&src).expect("assembles");
    prog.load(&mut mem).unwrap();
    // MSC main loop at 0x140: sample C6 for display tasks (@SEC), then
    // supervise the keyboard: when the CPU has consumed KEYBUF (0xFF),
    // restart the poll BCE (@SIO) — the handshake that stops the poller
    // racing the CPU.
    mem.load_halfwords(
        0x140,
        &[
            0xE600, // @SEC
            0xC000, // @DLY (alignment)
            0xF408, 0x1800, // @LH KEYBUF
            0x700C, // @X =0x000000FF (zero iff consumed)
            0x2303, // @BC nonzero -> skip the restart
            0xF400, 0x0152, // @LF poll-BCE busy mask
            0xE400, // @SIO
            0xC000, // @DLY (alignment)
            0xF000, 0x0140, // @BU main loop
        ],
    )
    .unwrap();
    mem.write_f(0x150, 0x0000_00FF).unwrap();
    mem.write_f(0x152, 0x0400_0000).unwrap(); // BCE index 4 (keyboard)
    // MSC echo program at 0x160: 4 save fullwords, then @LF mask, @SIO,
    // @REC back into the saved state.
    mem.load_halfwords(0x168, &[0xF400, 0x0170, 0xE400, 0xA000 | (0x7F5)]).unwrap();
    mem.write_f(0x170, 0x0200_0000).unwrap(); // busy mask: BCE index 5 (display)
    // BCE B (display send) at 0x1A0: base TEXT, #MOUT 1 char (command
    // fullword at 0x1A4 = DISPCMD), wait, loop.
    let disp_cmd0: u32 = (DEU_IUA << 19) | (2 << 16) | 1; // cell 0, 1 char
    mem.load_halfwords(
        0x1A0,
        &[
            0xF200, TEXT as u16, // #LBR TEXT
            0b11110101_00000000, 0, // #MOUT disp 0, 1 word
            (disp_cmd0 >> 16) as u16, disp_cmd0 as u16,
            0x0800, // #WAT
            0xC000, // #DLYI (even alignment)
            0xF000, 0x01A2, // #BU 0x1A2
        ],
    )
    .unwrap();
    // The CPU patches DISPCMD (0x1A4) before each send: pre-decrement so
    // the first bump lands on cell 0.
    mem.write_f(0x1A4, disp_cmd0.wrapping_sub(16)).unwrap();
    // BCE A (keyboard poll) at 0x200, as in the crew station.
    let poll_cmd: u32 = (DEU_IUA << 19) | (1 << 16) | 1;
    mem.load_halfwords(
        0x200,
        &[
            0xF200, KEYBUF as u16, // #LBR
            0xB000 | 100,          // #LTOI
            0xC000,                // #DLYI (alignment)
            0b11110001_00000000, 0, // #MIN 1 word -> KEYBUF
            (poll_cmd >> 16) as u16, poll_cmd as u16,
            0xE800,                // #SIB (key delivered)
            0x0800,                // #WAT (until the MSC restarts us)
            0xF000, 0x0204,        // #BU -> poll again
        ],
    )
    .unwrap();
    mem.write_h(KEYBUF, 0xFF).unwrap();

    let mut gpc = Gpc::new(mem);
    gpc.cpu.psw.ic = 0x100;
    {
        let mut iop = gpc.iop.borrow_mut();
        iop.halted = false;
        iop.msc.busy = true;
        iop.msc.pc = 0x140;
        iop.bces[DISPLAY_BUS].busy = true;
        iop.bces[DISPLAY_BUS].pc = 0x200;
        iop.mia_xmtr_enable = 0xFC00_0000;
        iop.mia_rcvr_enable = 0xFC00_0000;
    }
    // Layout: keyboard on bus 4 (BCE 4 polls), display on bus 5 (BCE 5
    // sends) — two DEU ports, echoing the real DEU's separate keyboard
    // and display channels through the DPS.
    {
        let mut iop = gpc.iop.borrow_mut();
        iop.bces[5].busy = false; // started by the MSC's @SIO
        iop.bces[5].pc = 0x1A0; // first run executes the #LBR; the loop
                                // re-enters at the #MOUT
    }
    let mut set = RedundantSet::new(vec![gpc]);
    let mut deu_kbd = Deu::new(4, DEU_IUA as u8, 4, 16);
    let deu_crt = Deu::new(5, DEU_IUA as u8, 4, 16);
    deu_kbd.type_keys(&[]);
    set.subsystems.push(Box::new(deu_kbd));
    set.subsystems.push(Box::new(deu_crt));

    for k in [key::OPS, 2, 0, 1] {
        set.subsystems[0]
            .as_any_mut()
            .downcast_mut::<Deu>()
            .unwrap()
            .press(k);
        set.run(300);
    }
    let crt = set.subsystems[1].as_any().downcast_ref::<Deu>().unwrap();
    assert_eq!(&crt.screen_text()[0][..4], "O201", "CPU-driven echo");
    assert_eq!(set.gpcs[0].iop.borrow().c6, 0, "C6 consumed by @SEC");
    assert!(set.dead[0].is_none());
}
