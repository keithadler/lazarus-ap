//! Instruction-level tests: floating point (IBM 85-C67-001 §8).
//!
//! Reference values come from the manual itself: the LFLI immediate table
//! (§8.21) fixes the bit patterns of 1.0..15.0 (0x41100000..0x41F00000),
//! from which 0.5 = 0x40800000, 16.0 = 0x42100000 follow by the §8.1/8.2
//! format definition.

mod common;
use common::*;
use lazarus_ap::Trap;

#[test]
fn lfli_ler_lflr_lfxr() {
    // LFLI value code n loads n.0 (§8.21); code 0 loads true zero.
    let mut c = cpu8k();
    exec1(&mut c, &[0b10001_010_1110_0011]); // LFLI 2,3
    assert_eq!(c.fpr[2], 0x4130_0000);
    // LER copies and sets CC from the fraction (§8.18).
    run_at(&mut c, 0x200, &[0b01111_001_11100_010], 1); // LER 1,2
    assert_eq!(c.fpr[1], 0x4130_0000);
    assert_eq!(c.psw.cc, CC_POS);
    // LFLR: GPR -> FPR; LFXR: FPR -> GPR (bit copies, §8.20/8.22).
    let mut c = cpu8k();
    c.set_r(3, 0xC120_0000);
    run_at(&mut c, 0x300, &[0b00101_100_11101_011], 1); // LFLR 4,3
    assert_eq!(c.fpr[4], 0xC120_0000);
    run_at(&mut c, 0x400, &[0b00100_101_11101_100], 1); // LFXR 5,4
    assert_eq!(c.r(5), 0xC120_0000);
}

#[test]
fn le_load_does_not_normalize_and_ccs_on_fraction() {
    let mut c = cpu8k();
    // unnormalized value: char 0x42, fraction 0x012345
    c.mem.write_f(0x400, 0x4201_2345).unwrap();
    exec1(&mut c, &[0b01111_001_11110_011, 0x0400]); // LE 1,=0x400
    assert_eq!(c.fpr[1], 0x4201_2345, "loads do not normalize (§8.3)");
    assert_eq!(c.psw.cc, CC_POS);
    // zero fraction with nonzero char: CC 00 though not true zero (§8.7)
    c.mem.write_f(0x400, 0x4200_0000).unwrap();
    run_at(&mut c, 0x200, &[0b01111_001_11110_011, 0x0400], 1);
    assert_eq!(c.psw.cc, CC_ZERO);
}

#[test]
fn ae_and_ser() {
    // 1.0 + 2.0 = 3.0 (storage operand via RS extended).
    let mut c = cpu8k();
    c.fpr[1] = 0x4110_0000;
    c.mem.write_f(0x400, 0x4120_0000).unwrap();
    exec1(&mut c, &[0b01010_001_11110_011, 0x0400]); // AE 1,=0x400
    assert_eq!(c.fpr[1], 0x4130_0000);
    assert_eq!(c.psw.cc, CC_POS);
    // AE SRS form scales the displacement for fullword operands.
    let mut c = cpu8k();
    c.fpr[1] = 0x4110_0000;
    c.set_r(0, 0x0400u32 << 16);
    c.mem.write_f(0x0406, 0x4110_0000).unwrap();
    exec1(&mut c, &[0b01010_001_000011_00]); // AE 1,d=3(0) -> EA 0x406
    assert_eq!(c.fpr[1], 0x4120_0000);
    // SER x,x: significance case — true zero, CC 00 (§8.8, mask off).
    let mut c = cpu8k();
    c.fpr[2] = 0x4550_1234;
    exec1(&mut c, &[0b01011_010_11100_010]); // SER 2,2
    assert_eq!(c.fpr[2], 0);
    assert_eq!(c.psw.cc, CC_ZERO);
    // 16.0 - 1.0 = 15.0 exercises prealignment + guard digit.
    let mut c = cpu8k();
    c.fpr[1] = 0x4210_0000;
    c.mem.write_f(0x400, 0x4110_0000).unwrap();
    exec1(&mut c, &[0b01011_001_11110_011, 0x0400]); // SE 1,=0x400
    assert_eq!(c.fpr[1], 0x41F0_0000);
    assert_eq!(c.psw.cc, CC_POS);
}

#[test]
fn cer_compare() {
    let mut c = cpu8k();
    c.fpr[1] = 0x4110_0000; // 1.0
    c.fpr[2] = 0x4120_0000; // 2.0
    exec1(&mut c, &[0b01001_001_11101_010]); // CER 1,2
    assert_eq!(c.psw.cc, CC_NEG);
    run_at(&mut c, 0x200, &[0b01001_010_11101_001], 1); // CER 2,1
    assert_eq!(c.psw.cc, CC_POS);
    // zero fractions equal regardless of sign/characteristic (§8.12)
    c.fpr[1] = 0xC500_0000;
    c.fpr[2] = 0x1200_0000;
    run_at(&mut c, 0x300, &[0b01001_001_11101_010], 1);
    assert_eq!(c.psw.cc, CC_ZERO);
}

#[test]
fn mer_even_pair_and_odd() {
    // 2.0 x 0.5 = 1.0; even R1 keeps the full-precision product in the
    // register pair (§8.25).
    let mut c = cpu8k();
    c.fpr[2] = 0x4120_0000;
    c.fpr[4] = 0x4080_0000;
    c.fpr[3] = 0xDEAD_BEEF;
    c.psw.cc = CC_NEG;
    exec1(&mut c, &[0b01100_010_11100_100]); // MER 2,4
    assert_eq!(c.fpr[2], 0x4110_0000);
    assert_eq!(c.fpr[3], 0, "even R1: pair receives the long product");
    assert_eq!(c.psw.cc, CC_NEG, "multiply leaves CC unchanged (§8.7)");
    // odd R1: single-register product, neighbor untouched
    let mut c = cpu8k();
    c.fpr[3] = 0x4120_0000;
    c.fpr[5] = 0x4080_0000;
    c.fpr[4] = 0xDEAD_BEEF;
    exec1(&mut c, &[0b01100_011_11100_101]); // MER 3,5
    assert_eq!(c.fpr[3], 0x4110_0000);
    assert_eq!(c.fpr[4], 0xDEAD_BEEF);
    // true zero forced when an operand has a zero fraction (§8.25)
    let mut c = cpu8k();
    c.fpr[3] = 0x4120_0000;
    c.fpr[5] = 0x7700_0000; // zero fraction
    exec1(&mut c, &[0b01100_011_11100_101]);
    assert_eq!(c.fpr[3], 0);
}

#[test]
fn der_divide_and_exceptions() {
    let mut c = cpu8k();
    c.fpr[1] = 0x4110_0000; // 1.0
    c.fpr[2] = 0x4120_0000; // 2.0
    exec1(&mut c, &[0b01101_001_11100_010]); // DER 1,2
    assert_eq!(c.fpr[1], 0x4080_0000); // 0.5
    // divide by zero fraction: suppressed, PE code 000C (§8.8/8.16)
    let mut c = cpu8k();
    c.fpr[1] = 0x4110_0000;
    c.fpr[2] = 0x4400_0000; // zero fraction
    let t = exec1_err(&mut c, &[0b01101_001_11100_010]);
    assert!(matches!(t, Trap::UninitializedInterrupt { code: 0x000C, .. }));
    assert_eq!(c.fpr[1], 0x4110_0000, "division suppressed, dividend kept");
}

#[test]
fn exponent_overflow_and_underflow() {
    // char 127 * char 127: exponent overflow, PE code 000B, operands
    // unchanged (§8.8).
    let mut c = cpu8k();
    c.fpr[1] = 0x7F10_0000;
    c.fpr[2] = 0x7F10_0000;
    let t = exec1_err(&mut c, &[0b01100_001_11100_010]); // MER 1,2 (odd R1)
    assert!(matches!(t, Trap::UninitializedInterrupt { code: 0x000B, .. }));
    assert_eq!(c.fpr[1], 0x7F10_0000);
    // underflow with mask off: true zero, no interrupt (§8.8)
    let mut c = cpu8k();
    c.fpr[1] = 0x0010_0000; // char 0
    c.fpr[2] = 0x0010_0000;
    exec1(&mut c, &[0b01100_001_11100_010]);
    assert_eq!(c.fpr[1], 0);
    // underflow with mask on: interrupt code 0009, no result written
    let mut c = cpu8k();
    c.psw.exp_underflow_mask = true;
    c.fpr[1] = 0x0010_0000;
    c.fpr[2] = 0x0010_0000;
    let t = exec1_err(&mut c, &[0b01100_001_11100_010]);
    assert!(matches!(t, Trap::UninitializedInterrupt { code: 0x0009, .. }));
    assert_eq!(c.fpr[1], 0x0010_0000);
}

#[test]
fn long_operands_round_trip() {
    // LED/STED move doubleword operands through the register pair; AEDR
    // adds with 14-digit precision (§8.9/8.17/8.28).
    let mut c = cpu8k();
    c.mem.write_f(0x400, 0x4110_0000).unwrap();
    c.mem.write_f(0x402, 0x0000_0001).unwrap(); // 1.0 + 16^-14*16
    exec1(&mut c, &[0b01111_010_11111_011, 0x0400]); // LED 2,=0x400
    assert_eq!((c.fpr[2], c.fpr[3]), (0x4110_0000, 0x0000_0001));
    assert_eq!(c.psw.cc, CC_POS);
    // AEDR 2,4 with the 4-5 pair holding the same value: (1+eps)*2, so
    // the low-order bit doubles too.
    c.fpr[4] = 0x4110_0000;
    c.fpr[5] = 0x0000_0001;
    run_at(&mut c, 0x200, &[0b01010_010_11101_100], 1); // AEDR 2,4
    assert_eq!((c.fpr[2], c.fpr[3]), (0x4120_0000, 0x0000_0002));
    run_at(&mut c, 0x300, &[0b00111_010_11111_011, 0x0500], 1); // STED 2,=0x500
    assert_eq!(c.mem.read_f(0x500).unwrap(), 0x4120_0000);
    assert_eq!(c.mem.read_f(0x502).unwrap(), 0x0000_0002);
    // CEDR: long compare distinguishes the low digits short would miss
    c.fpr[4] = 0x4120_0000;
    c.fpr[5] = 0x0000_0000;
    run_at(&mut c, 0x400, &[0b00011_010_11101_100], 1); // CEDR 2,4
    assert_eq!(c.psw.cc, CC_POS);
}

#[test]
fn cvfl_and_cvfx() {
    // Fixed 3 (binary point between bits 15/16: 0x0003_0000) -> 3.0 (§8.14).
    let mut c = cpu8k();
    c.set_r(2, 0x0003_0000);
    exec1(&mut c, &[0b00111_001_11101_010]); // CVFL 1,2
    assert_eq!(c.fpr[1], 0x4130_0000);
    assert_eq!(c.psw.cc, CC_POS);
    // -1.0 fixed = 0xFFFF_0000
    let mut c = cpu8k();
    c.set_r(2, 0xFFFF_0000);
    exec1(&mut c, &[0b00111_001_11101_010]);
    assert_eq!(c.fpr[1], 0xC110_0000);
    assert_eq!(c.psw.cc, CC_NEG);
    // CVFX: 3.0 -> 0x0003_0000; CC on bits 0-15 (§8.13)
    let mut c = cpu8k();
    c.fpr[2] = 0x4130_0000;
    exec1(&mut c, &[0b00111_001_11100_010]); // CVFX 1,2
    assert_eq!(c.r(1), 0x0003_0000);
    assert_eq!(c.psw.cc, CC_POS);
    // fractional value 0.5 -> 0x0000_8000, CC 00 (integer bits zero)
    let mut c = cpu8k();
    c.fpr[2] = 0x4080_0000;
    exec1(&mut c, &[0b00111_001_11100_010]);
    assert_eq!(c.r(1), 0x0000_8000);
    assert_eq!(c.psw.cc, CC_ZERO);
    // convert overflow: 16^5 is outside the fixed range (§8.13)
    let mut c = cpu8k();
    c.fpr[2] = 0x4510_0000;
    let t = exec1_err(&mut c, &[0b00111_001_11100_010]);
    assert!(matches!(t, Trap::UninitializedInterrupt { code: 0x000A, .. }));
}

#[test]
fn mvs_limiter() {
    // §8.23 limiter: value in FPR R1, upper limit in FPR (R1+1), lower
    // limit in storage.
    let ops = [0b01100_010_11111_011u16, 0x0400]; // MVS 2,=0x400
    // within limits: R1 unchanged, CC 00
    let mut c = cpu8k();
    c.fpr[2] = 0x4150_0000; // 5.0
    c.fpr[3] = 0x41A0_0000; // upper 10.0
    c.mem.write_f(0x400, 0x4120_0000).unwrap(); // lower 2.0
    exec1(&mut c, &ops);
    assert_eq!(c.fpr[2], 0x4150_0000);
    assert_eq!(c.psw.cc, CC_ZERO);
    // above upper: clamped to upper, CC 01
    let mut c = cpu8k();
    c.fpr[2] = 0x41C0_0000; // 12.0
    c.fpr[3] = 0x41A0_0000;
    c.mem.write_f(0x400, 0x4120_0000).unwrap();
    exec1(&mut c, &ops);
    assert_eq!(c.fpr[2], 0x41A0_0000);
    assert_eq!(c.psw.cc, CC_POS);
    // below lower: clamped to lower, CC 11
    let mut c = cpu8k();
    c.fpr[2] = 0x4110_0000; // 1.0
    c.fpr[3] = 0x41A0_0000;
    c.mem.write_f(0x400, 0x4120_0000).unwrap();
    exec1(&mut c, &ops);
    assert_eq!(c.fpr[2], 0x4120_0000);
    assert_eq!(c.psw.cc, CC_NEG);
}

#[test]
fn lecr_complement() {
    let mut c = cpu8k();
    c.fpr[2] = 0x4110_0000;
    exec1(&mut c, &[0b01111_001_11101_010]); // LECR 1,2
    assert_eq!(c.fpr[1], 0xC110_0000);
    assert_eq!(c.psw.cc, CC_NEG);
    // zero fraction loads as true zero (§8.19)
    let mut c = cpu8k();
    c.fpr[2] = 0x4400_0000;
    exec1(&mut c, &[0b01111_001_11101_010]);
    assert_eq!(c.fpr[1], 0);
    assert_eq!(c.psw.cc, CC_ZERO);
}

#[test]
fn ste_stores_without_cc() {
    let mut c = cpu8k();
    c.fpr[5] = 0xC134_5678;
    c.psw.cc = CC_POS;
    exec1(&mut c, &[0b00111_101_11110_011, 0x0410]); // STE 5,=0x410
    assert_eq!(c.mem.read_f(0x410).unwrap(), 0xC134_5678);
    assert_eq!(c.psw.cc, CC_POS, "stores leave the CC unchanged (§8.7)");
}
