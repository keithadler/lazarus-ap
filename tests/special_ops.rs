//! Tests for the §9 special operations, storage protection (§2.4), and
//! the instruction monitor (§2.4.1).

mod common;
use common::*;
use lazarus_ap::{Psw, Trap};

#[test]
fn scal_sret_round_trip() {
    // §9.7/9.8: SCAL saves PSW word 0 + all 8 GPRs in an 18-halfword stack
    // frame, updates the SSD (PTR += INC, INC = 18), and branches; SRET
    // conditionally restores everything.
    let mut c = cpu8k();
    // SSD in R1: sector via DSE (bit 0 = 0, DSE 0 = sector 0), PTR offset
    // 0x0600, INC 0 (empty stack).
    c.set_r(1, 0x0600_0000);
    for n in 2..8u8 {
        c.set_r(n, 0x1000_0000 + n as u32);
    }
    c.psw.cc = CC_POS;
    // SCAL 1,=0x0500
    run_at(&mut c, 0x100, &[0b11010_001_11111_011, 0x0500], 1);
    assert_eq!(c.psw.ic, 0x0500, "branched to the subroutine");
    assert_eq!(c.r(1), 0x0600_0012, "SSD: PTR at frame, INC = 18");
    // frame: PSW word 0 (return address 0x102, CC 01) then R0..R7
    let w0 = c.mem.read_f(0x600).unwrap();
    assert_eq!(w0 >> 16, 0x0102);
    assert_eq!((w0 >> 14) & 3, 0b01);
    assert_eq!(c.mem.read_f(0x600 + 2 + 2).unwrap(), 0x0600_0000, "saved R1 = old SSD");
    assert_eq!(c.mem.read_f(0x600 + 2 + 14).unwrap(), 0x1000_0007);
    // subroutine trashes registers and the CC, then returns: SRET 7,1
    c.set_r(2, 0xDEAD_BEEF);
    c.psw.cc = CC_NEG;
    run_at(&mut c, 0x500, &[0b10010_111_11101_001], 1);
    assert_eq!(c.psw.ic, 0x0102, "returned past the SCAL");
    assert_eq!(c.psw.cc, CC_POS, "caller's CC restored from the stack");
    assert_eq!(c.r(1), 0x0600_0000, "SSD restored");
    assert_eq!(c.r(2), 0x1000_0002, "registers restored");
    // conditional: SRET with a failing mask does nothing
    let mut c = cpu8k();
    c.psw.cc = CC_NEG;
    c.set_r(1, 0x0600_0000);
    run_at(&mut c, 0x100, &[0b10010_001_11101_001], 1); // SRET 1 (CC=01 test)
    assert_eq!(c.psw.ic, 0x101, "no branch, no restore");
}

#[test]
fn mvh_block_move() {
    // §9.4: count in R1 bits 16-31; destination offset in R1 bits 1-15
    // (sector from DSE/DSR by bit 0); source offset + DSR in R2.
    // The count decrements before each move: offsets count-1..0 (the
    // Figure 9-1 order as built; confirmed against yaGPC2 — and against
    // a real HALSFC program whose constant table's first halfword was
    // dropped by the off-by-one this replaced).
    let mut c = cpu8k();
    for i in 0..4u32 {
        c.mem.write_h(0x500 + i, 0x1110 + i as u16).unwrap();
    }
    c.set_r(1, (0x0600u32 << 16) | 4); // dest 0x600, count 4
    c.set_r(2, 0x0500u32 << 16); // source 0x500, bit 0 = 0 (sector 0)
    exec1(&mut c, &[0b01101_001_11101_010]); // MVH 1,2
    for i in 0..4u32 {
        assert_eq!(c.mem.read_h(0x600 + i).unwrap(), 0x1110 + i as u16);
    }
    assert_eq!(c.r(1), 0x0600_0000, "count decremented to zero");
    // zero/negative count: no move (§9.4)
    let mut c = cpu8k();
    c.mem.write_h(0x500, 0xAAAA).unwrap();
    c.set_r(1, 0x0600u32 << 16);
    c.set_r(2, 0x0500u32 << 16);
    exec1(&mut c, &[0b01101_001_11101_010]);
    assert_eq!(c.mem.read_h(0x600).unwrap(), 0);
}

#[test]
fn ispb_and_store_protection() {
    // §9.2 + §2.4: ISPB sets/resets protection; a store to a protected
    // halfword takes a program interrupt (code 0007) and does not occur.
    let mut c = cpu8k();
    // ISPB 2,=0x400 (M1=010: set halfword)
    run_at(&mut c, 0x100, &[0b11101_010_11111_011, 0x0400], 1);
    assert!(c.mem.is_protected(0x400));
    // STH into the protected halfword: PE 0007, store suppressed
    c.set_r(5, 0xBEEF_0000);
    let t = try_run_at(&mut c, 0x200, &[0b10111_101_11110_011, 0x0400], 1).unwrap_err();
    assert!(matches!(t, Trap::UninitializedInterrupt { code: 0x0007, .. }));
    assert_eq!(c.mem.read_h(0x400).unwrap(), 0, "store did not occur");
    // M1=000 resets; store then succeeds
    run_at(&mut c, 0x300, &[0b11101_000_11111_011, 0x0400], 1);
    assert!(!c.mem.is_protected(0x400));
    run_at(&mut c, 0x400, &[0b10111_101_11110_011, 0x0400], 1);
    assert_eq!(c.mem.read_h(0x400).unwrap(), 0xBEEF);
    // M1=011 protects both halfwords of the fullword (EA low bit ignored)
    let mut c = cpu8k();
    run_at(&mut c, 0x100, &[0b11101_011_11111_011, 0x0501], 1);
    assert!(c.mem.is_protected(0x500) && c.mem.is_protected(0x501));
    // M1 with bit 5 set is illegal (§9.2)
    let mut c = cpu8k();
    let t = try_run_at(&mut c, 0x100, &[0b11101_100_11111_011, 0x0400], 1).unwrap_err();
    assert!(matches!(t, Trap::UninitializedInterrupt { code: 0x0000, .. }));
    // ISPB is privileged
    let mut c = cpu8k();
    c.psw.problem_state = true;
    let t = try_run_at(&mut c, 0x100, &[0b11101_010_11111_011, 0x0400], 1).unwrap_err();
    assert!(matches!(t, Trap::UninitializedInterrupt { code: 0x0001, .. }));
}

#[test]
fn instruction_monitor() {
    // §2.4.1: with PSW bit 34 set, executing an unprotected instruction
    // interrupts through 0070/0074 with the IC left at the offender.
    let mut c = cpu8k();
    let mut handler = Psw::default();
    handler.ic = 0x0300;
    c.mem.write_f(0x74, handler.word0()).unwrap();
    c.mem.write_f(0x76, handler.word1()).unwrap();
    c.psw.sys_mask = 0b0010_0000; // bit 34
    c.mem.load_halfwords(0x100, &[0b00000_001_11100_010]).unwrap(); // AR
    c.psw.ic = 0x100;
    c.step().unwrap();
    assert_eq!(c.psw.ic, 0x0300, "monitor interrupt taken");
    assert_eq!(c.mem.read_f(0x70).unwrap() >> 16, 0x0100, "IC at offender");
    assert_eq!(c.r(1), 0, "instruction did not execute");
    // protected code executes normally under the monitor
    let mut c = cpu8k();
    c.psw.sys_mask = 0b0010_0000;
    c.set_r(2, 7);
    c.mem.load_halfwords(0x100, &[0b00000_001_11100_010]).unwrap();
    c.mem.set_protected(0x100, true).unwrap();
    c.psw.ic = 0x100;
    c.step().unwrap();
    assert_eq!(c.r(1), 7);
}

#[test]
fn lxa_stxa_ldm_stdm() {
    // §9.12: LXA loads R1 bits 1-15 + its DSE from an address constant.
    let mut c = cpu8k();
    c.mem.write_f(0x400, 0x8123_0005).unwrap(); // addr bits 1-15 = 0x123, DSE 5
    exec1(&mut c, &[0b01000_010_11111_011, 0x0400]); // LXA 2,=0x400
    assert_eq!(c.r(2), 0x0123_0000);
    assert_eq!(c.dse[0][2], 5);
    // §9.14: STXA stores bit0=1, R1 bits 1-15, dest bits 20-27 kept, DSE.
    c.set_r(2, 0x0456_FFFF);
    c.mem.write_f(0x402, 0xFFFF_FFFF).unwrap();
    run_at(&mut c, 0x200, &[0b10100_010_11111_011, 0x0402], 1); // STXA 2,=0x402
    assert_eq!(c.mem.read_f(0x402).unwrap(), 0x8456_0FF5);
    // §9.13/9.15: LDM/STDM move the R0-R3 DSEs as packed nibbles.
    let mut c = cpu8k();
    c.mem.write_f(0x400, 0x0102_0304).unwrap();
    exec1(&mut c, &[0b01101_000_11111_011, 0x0400]); // LDM =0x400
    assert_eq!([c.dse[0][0], c.dse[0][1], c.dse[0][2], c.dse[0][3]], [1, 2, 3, 4]);
    run_at(&mut c, 0x200, &[0b10010_000_11111_011, 0x0402], 1); // STDM =0x402
    assert_eq!(c.mem.read_f(0x402).unwrap(), 0x0102_0304);
    // a DSE now takes part in expanded addressing (§2.2.9)
    let mut c = cpu_full();
    c.mem.write_f(0x400, 0x0000_0002).unwrap(); // DSE for R3 = 2
    run_at(&mut c, 0x100, &[0b01101_000_11111_011, 0x0400], 1); // LDM
    c.set_r(3, 0x0100u32 << 16);
    c.mem.write_f((2 << 15) | 0x0104, 777).unwrap();
    // L 1,4(3): SRS fullword, base R3 -> sector from DSE[3]
    run_at(&mut c, 0x200, &[0b00011_001_000010_11], 1);
    assert_eq!(c.r(1), 777);
}

#[test]
fn tsb_test_and_set_bits() {
    // §9.11: three-state test then OR the mask in.
    let mut c = cpu8k();
    c.set_r(1, 0x0400u32 << 16);
    c.mem.write_h(0x405, 0b1000).unwrap();
    exec1(&mut c, &[0b10110_111_000101_01, 0b0011]); // TSB 5(1),0b0011
    assert_eq!(c.psw.cc, CC_ZERO, "selected bits were zero");
    assert_eq!(c.mem.read_h(0x405).unwrap(), 0b1011);
    run_at(&mut c, 0x200, &[0b10110_111_000101_01, 0b0011], 1);
    assert_eq!(c.psw.cc, CC_POS, "now all ones: lock held");
}

#[test]
fn fullword_indirect_data_pointer() {
    // §2.2.8 step 6 / Figure 2-15: X=0, IA=1, I=1 — fullword pointer with
    // automatic storage modification.
    let mut c = cpu8k();
    c.set_r(1, 0x0400u32 << 16);
    // pointer at 0x410: address 0x0500, modifier +2
    c.mem.write_f(0x410, 0x0500_0002).unwrap();
    c.mem.write_f(0x500, 4242).unwrap();
    // L 2,0x10(0,1) with IA=1, I=1
    exec1(
        &mut c,
        &[0b00011_010_11110_101, (1 << 12) | (1 << 11) | 0x010],
    );
    assert_eq!(c.r(2), 4242, "loads through the pointer's address half");
    assert_eq!(
        c.mem.read_f(0x410).unwrap(),
        0x0502_0002,
        "modifier added to the address afterwards (Figure 2-15)"
    );
}

#[test]
fn pc_program_controlled_io() {
    use lazarus_ap::{IoSubsystem, PcResponse};
    // §3.3: CW in R2 (bit 0 selects input/output), data in R1; CC 00 on
    // success, 01 on interface timeout. Privileged.
    struct Loopback {
        last: u32,
    }
    impl IoSubsystem for Loopback {
        fn pc(&mut self, cw: u32, data: Option<u32>) -> PcResponse {
            match data {
                Some(v) => {
                    self.last = v;
                    let _ = cw;
                    PcResponse::OutputAccepted
                }
                None => PcResponse::Input(self.last.wrapping_add(1)),
            }
        }
    }
    let mut c = cpu8k();
    c.io = Some(Box::new(Loopback { last: 0 }));
    // output: CW bit 0 = 1
    c.set_r(2, 0x8000_0123);
    c.set_r(1, 0xCAFE_0000);
    exec1(&mut c, &[0b11011_001_11101_010]); // PC 1,2
    assert_eq!(c.psw.cc, CC_ZERO);
    // input: CW bit 0 = 0 -> loads R1
    c.set_r(2, 0x0000_0123);
    run_at(&mut c, 0x200, &[0b11011_001_11101_010], 1);
    assert_eq!(c.r(1), 0xCAFE_0001);
    assert_eq!(c.psw.cc, CC_ZERO);
    // no subsystem attached: handshake timeout, CC 01 (§3.3)
    let mut c = cpu8k();
    c.set_r(2, 0x0000_0123);
    exec1(&mut c, &[0b11011_001_11101_010]);
    assert_eq!(c.psw.cc, CC_POS);
    // privileged (§3.3 programming note)
    let mut c = cpu8k();
    c.psw.problem_state = true;
    let t = exec1_err(&mut c, &[0b11011_001_11101_010]);
    assert!(matches!(t, Trap::UninitializedInterrupt { code: 0x0001, .. }));
}
