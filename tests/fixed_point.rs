//! Instruction-level tests: fixed-point arithmetic (IBM 85-C67-001 §4).
//!
//! Each test hand-encodes the instruction (proving the §13 op-code
//! assignment and field layout) and asserts the documented effect on
//! registers/storage, the condition code, and the carry/overflow
//! indicators.

mod common;
use common::*;

// ---------- ADD family (§4.1-4.4) ----------

#[test]
fn ar_add_register() {
    // AR R1,R2: op 00000, bits 8-12 = 11100. AR 3,5
    let mut c = cpu8k();
    c.set_r(3, 5);
    c.set_r(5, 7);
    exec1(&mut c, &[0b00000_011_11100_101]);
    assert_eq!(c.r(3), 12);
    assert_eq!(c.psw.cc, CC_POS);
    assert!(!c.psw.carry);
    assert!(!c.psw.overflow);
}

#[test]
fn ar_zero_result_sets_carry_and_cc_zero() {
    // 0xFFFFFFFF + 1 = 0 with carry out of the high-order bit (§4.1).
    let mut c = cpu8k();
    c.set_r(1, 0xFFFF_FFFF);
    c.set_r(2, 1);
    exec1(&mut c, &[0b00000_001_11100_010]);
    assert_eq!(c.r(1), 0);
    assert_eq!(c.psw.cc, CC_ZERO);
    assert!(c.psw.carry);
    assert!(!c.psw.overflow);
}

#[test]
fn ar_overflow_is_sticky() {
    // Magnitude too large: overflow indicator set; a later non-overflowing
    // add must not clear it (§4.1: "If the overflow indicator already
    // contains a one, it is not altered").
    let mut c = cpu8k();
    c.set_r(1, 0x7FFF_FFFF);
    c.set_r(2, 1);
    exec1(&mut c, &[0b00000_001_11100_010]);
    assert_eq!(c.r(1), 0x8000_0000);
    assert_eq!(c.psw.cc, CC_NEG);
    assert!(c.psw.overflow);
    run_at(&mut c, 0x200, &[0b00000_001_11100_010], 1); // + 1 again
    assert!(c.psw.overflow, "overflow indicator must be sticky");
}

#[test]
fn a_srs_fullword() {
    // A R1,D2(B2): SRS displacement is fullword-aligned (scaled x2,
    // Figure 2-8). A 2,disp 3(B2=1): EA = base + 6.
    let mut c = cpu8k();
    c.set_r(1, 0x0000_0400 << 16); // base 0x400 in bits 0-15
    c.set_r(2, 100);
    c.mem.write_f(0x406, 23).unwrap();
    exec1(&mut c, &[0b00000_010_000011_01]);
    assert_eq!(c.r(2), 123);
    assert_eq!(c.psw.cc, CC_POS);
}

#[test]
fn a_rs_extended() {
    // A R1,D2(B2) extended: B2=11 means the 16-bit displacement is the
    // address (§2.2.8).
    let mut c = cpu8k();
    c.set_r(4, 1);
    c.mem.write_f(0x500, 0xFFFF_FFFE).unwrap(); // -2
    exec1(&mut c, &[0b00000_100_11110_011, 0x0500]);
    assert_eq!(c.r(4), 0xFFFF_FFFF);
    assert_eq!(c.psw.cc, CC_NEG);
}

#[test]
fn ah_develops_halfword_high() {
    // AH: halfword operand becomes the most significant 16 bits with 16
    // low-order zeros (§4.0, §4.2). AH 1,4(0)
    let mut c = cpu8k();
    c.set_r(0, 0x0300 << 16);
    c.set_r(1, 0x0001_0000);
    c.mem.write_h(0x304, 0x0002).unwrap();
    exec1(&mut c, &[0b10000_001_000100_00]);
    assert_eq!(c.r(1), 0x0003_0000);
    assert_eq!(c.psw.cc, CC_POS);
}

#[test]
fn ahi_immediate() {
    // AHI R2,data: op 10110, OPX=000, bits 8-12=11100 (§2.2.7, §13).
    let mut c = cpu8k();
    c.set_r(6, 0x0005_1234);
    exec1(&mut c, &[0b10110_000_11100_110, 0xFFFB]); // + (-5 in bits 0-15)
    assert_eq!(c.r(6), 0x0000_1234);
    assert_eq!(c.psw.cc, CC_POS); // result not zero: low bits remain
}

#[test]
fn ast_adds_to_storage() {
    // AST: register + storage -> storage; register unchanged (§4.4).
    let mut c = cpu8k();
    c.set_r(2, 10);
    c.mem.write_f(0x600, 32).unwrap();
    exec1(&mut c, &[0b00000_010_11111_011, 0x0600]);
    assert_eq!(c.mem.read_f(0x600).unwrap(), 42);
    assert_eq!(c.r(2), 10);
    assert_eq!(c.psw.cc, CC_POS);
}

// ---------- SUBTRACT family (§4.28-4.30) ----------

#[test]
fn sr_subtract() {
    // 7 - 5 = 2: subtraction adds the ones complement + 1, so a carry out
    // means no borrow (§4.28).
    let mut c = cpu8k();
    c.set_r(1, 7);
    c.set_r(2, 5);
    exec1(&mut c, &[0b00001_001_11100_010]);
    assert_eq!(c.r(1), 2);
    assert_eq!(c.psw.cc, CC_POS);
    assert!(c.psw.carry);
    // 5 - 7 = -2, no carry out
    let mut c = cpu8k();
    c.set_r(1, 5);
    c.set_r(2, 7);
    exec1(&mut c, &[0b00001_001_11100_010]);
    assert_eq!(c.r(1), 0xFFFF_FFFE);
    assert_eq!(c.psw.cc, CC_NEG);
    assert!(!c.psw.carry);
}

#[test]
fn s_overflow() {
    // 0x80000000 - 1 overflows (§4.28 magnitude rule).
    let mut c = cpu8k();
    c.set_r(1, 0x8000_0000);
    c.set_r(2, 1);
    exec1(&mut c, &[0b00001_001_11100_010]);
    assert_eq!(c.r(1), 0x7FFF_FFFF);
    assert!(c.psw.overflow);
}

#[test]
fn sh_halfword() {
    let mut c = cpu8k();
    c.set_r(1, 0x0005_0000);
    c.set_r(0, 0x0300 << 16);
    c.mem.write_h(0x302, 0x0002).unwrap();
    exec1(&mut c, &[0b10001_001_000010_00]); // SH 1,2(0)
    assert_eq!(c.r(1), 0x0003_0000);
    assert_eq!(c.psw.cc, CC_POS);
}

#[test]
fn sst_subtracts_register_from_storage() {
    // §4.29: R1 subtracted FROM the second operand; result to storage.
    let mut c = cpu8k();
    c.set_r(3, 2);
    c.mem.write_f(0x700, 10).unwrap();
    exec1(&mut c, &[0b00001_011_11111_011, 0x0700]);
    assert_eq!(c.mem.read_f(0x700).unwrap(), 8);
    assert_eq!(c.r(3), 2);
    assert_eq!(c.psw.cc, CC_POS);
}

// ---------- COMPARE family (§4.5-4.9) ----------

#[test]
fn cr_compare_signed() {
    // CC: 00 equal, 11 R1 less, 01 R1 greater (§4.5); algebraic compare;
    // indicators unchanged.
    let mut c = cpu8k();
    c.psw.carry = true;
    c.psw.overflow = true;
    c.set_r(1, 0xFFFF_FFFF); // -1
    c.set_r(2, 1);
    exec1(&mut c, &[0b00010_001_11100_010]);
    assert_eq!(c.psw.cc, CC_NEG); // -1 < 1 algebraically
    assert!(c.psw.carry && c.psw.overflow, "indicators must be unchanged");
    c.set_r(2, 0xFFFF_FFFF);
    run_at(&mut c, 0x200, &[0b00010_001_11100_010], 1);
    assert_eq!(c.psw.cc, CC_ZERO);
    c.set_r(1, 3);
    c.set_r(2, 2);
    run_at(&mut c, 0x300, &[0b00010_001_11100_010], 1);
    assert_eq!(c.psw.cc, CC_POS);
}

#[test]
fn ch_and_chi() {
    // CH: all 32 bits of the developed fullword participate (§4.7), so
    // R1=1 compares less than developed 0x00010000.
    let mut c = cpu8k();
    c.set_r(1, 1);
    c.set_r(0, 0x0300u32 << 16);
    c.mem.write_h(0x301, 0x0001).unwrap();
    exec1(&mut c, &[0b10010_001_000001_00]); // CH 1,1(0)
    assert_eq!(c.psw.cc, CC_NEG);
    // CHI: op 10110 OPX=101
    let mut c = cpu8k();
    c.set_r(2, 0x0005_0000);
    exec1(&mut c, &[0b10110_101_11100_010, 0x0005]);
    assert_eq!(c.psw.cc, CC_ZERO);
}

#[test]
fn cist_immediate_vs_storage() {
    // §4.9: CC 11 = immediate less than the halfword storage operand.
    let mut c = cpu8k();
    c.set_r(1, 0x0400u32 << 16);
    c.mem.write_h(0x402, 7).unwrap();
    // CIST 2(1),5: op 10110 OPX=101 SI form
    exec1(&mut c, &[0b10110_101_000010_01, 5]);
    assert_eq!(c.psw.cc, CC_NEG);
}

#[test]
fn cbl_compare_between_limits() {
    // §4.6: R1 bits 0-15 address the operand; R2 bits 0-15 address a
    // fullword upper(bits 0-15)/lower(16-31) limits pair; CC 00 within,
    // 01 above, 11 below; then modifiers (bits 16-31) advance both
    // addresses.
    let mut c = cpu8k();
    c.mem.write_h(0x500, 10).unwrap(); // operand
    c.mem.write_h(0x600, 20).unwrap(); // upper limit
    c.mem.write_h(0x601, 5).unwrap(); // lower limit
    c.set_r(1, 0x0500_0002); // operand addr, modifier +2
    c.set_r(2, 0x0600_0000);
    exec1(&mut c, &[0b00001_001_11101_010]); // CBL 1,2 (RR-alt of op 00001)
    assert_eq!(c.psw.cc, CC_ZERO);
    assert_eq!(c.r(1), 0x0502_0002, "modifier advances the address half");
    assert_eq!(c.r(2), 0x0600_0000);
    // above upper
    c.mem.write_h(0x502, 21).unwrap();
    run_at(&mut c, 0x200, &[0b00001_001_11101_010], 1);
    assert_eq!(c.psw.cc, CC_POS);
    // below lower
    c.set_r(1, 0x0503_0000);
    c.mem.write_h(0x503, 4).unwrap();
    run_at(&mut c, 0x300, &[0b00001_001_11101_010], 1);
    assert_eq!(c.psw.cc, CC_NEG);
}

// ---------- MULTIPLY / DIVIDE (§4.10, 4.21-4.24) ----------

#[test]
fn mr_fractional_multiply_even_pair() {
    // 0.5 x 0.5 = 0.25: fractions, 64-bit product in even/odd pair
    // (§4.21). CC not changed.
    let mut c = cpu8k();
    c.psw.cc = CC_NEG;
    c.set_r(2, 0x4000_0000);
    c.set_r(4, 0x4000_0000);
    exec1(&mut c, &[0b01000_010_11100_100]); // MR 2,4
    assert_eq!(c.r(2), 0x2000_0000);
    assert_eq!(c.r(3), 0);
    assert_eq!(c.psw.cc, CC_NEG, "CC not changed by multiply");
}

#[test]
fn mr_odd_r1_keeps_high_half_only() {
    let mut c = cpu8k();
    c.set_r(3, 0x4000_0000);
    c.set_r(5, 0xC000_0000); // -0.5
    c.set_r(4, 0xDEAD_BEEF);
    exec1(&mut c, &[0b01000_011_11100_101]); // MR 3,5
    assert_eq!(c.r(3), 0xE000_0000); // -0.25
    assert_eq!(c.r(4), 0xDEAD_BEEF, "R1+1 untouched when R1 odd");
}

#[test]
fn mr_minus_one_squared_overflows() {
    // §4.21: overflow indicator set when -1 x -1.
    let mut c = cpu8k();
    c.set_r(2, 0x8000_0000);
    c.set_r(4, 0x8000_0000);
    exec1(&mut c, &[0b01000_010_11100_100]);
    assert!(c.psw.overflow);
}

#[test]
fn mh_halfword_fraction() {
    // 0.5 x 0.5 = 0.25 as a 32-bit fraction into all of R1 (§4.22).
    let mut c = cpu8k();
    c.set_r(1, 0x4000_FFFF); // low half irrelevant
    c.set_r(0, 0x0300u32 << 16);
    c.mem.write_h(0x301, 0x4000).unwrap();
    exec1(&mut c, &[0b10101_001_000001_00]); // MH 1,1(0)
    assert_eq!(c.r(1), 0x2000_0000);
}

#[test]
fn mhi_immediate_fraction() {
    let mut c = cpu8k();
    c.set_r(5, 0x4000_0000);
    exec1(&mut c, &[0b10110_111_11100_101, 0x2000]); // MHI 5,0.25
    assert_eq!(c.r(5), 0x1000_0000); // 0.125
}

#[test]
fn mih_integer_halfword() {
    // §4.24: integer product to bits 0-15, bits 16-31 zeroed.
    let mut c = cpu8k();
    c.set_r(1, (3u32 << 16) | 0xABCD);
    c.mem.write_h(0x800, 4).unwrap();
    exec1(&mut c, &[0b10011_001_11111_011, 0x0800]); // MIH 1,0x800
    assert_eq!(c.r(1), 12 << 16);
    assert!(!c.psw.overflow);
    // 300 x 300 overflows a signed halfword
    let mut c = cpu8k();
    c.set_r(1, 300u32 << 16);
    c.mem.write_h(0x800, 300).unwrap();
    exec1(&mut c, &[0b10011_001_11111_011, 0x0800]);
    assert_eq!(c.r(1), ((90000u32 & 0xFFFF) << 16));
    assert!(c.psw.overflow);
}

#[test]
fn dr_fractional_divide() {
    // 0.25 / 0.5 = 0.5. Even R1: 64-bit dividend in R0:R1 (§4.10).
    let mut c = cpu8k();
    c.set_r(0, 0x2000_0000);
    c.set_r(1, 0);
    c.set_r(4, 0x4000_0000);
    c.psw.cc = CC_POS;
    exec1(&mut c, &[0b01001_000_11100_100]); // DR 0,4
    assert_eq!(c.r(0), 0x4000_0000);
    assert_eq!(c.psw.cc, CC_POS, "CC not changed by divide");
    assert!(!c.psw.overflow);
}

#[test]
fn dr_odd_r1_appends_zeros() {
    // Odd R1: dividend developed by appending 32 low-order zeros (§4.10).
    let mut c = cpu8k();
    c.set_r(3, 0x2000_0000); // 0.25
    c.set_r(5, 0x4000_0000); // 0.5
    exec1(&mut c, &[0b01001_011_11100_101]); // DR 3,5
    assert_eq!(c.r(3), 0x4000_0000);
}

#[test]
fn dr_overflow_and_divide_by_zero() {
    // Quotient magnitude >= 1 overflows; registers left unchanged
    // (documented deterministic choice for the manual's "indeterminate").
    let mut c = cpu8k();
    c.set_r(3, 0x4000_0000);
    c.set_r(5, 0x4000_0000); // 0.5/0.5 = 1.0: unrepresentable
    exec1(&mut c, &[0b01001_011_11100_101]);
    assert!(c.psw.overflow);
    assert_eq!(c.r(3), 0x4000_0000);
    let mut c = cpu8k();
    c.set_r(3, 0x4000_0000);
    c.set_r(5, 0);
    exec1(&mut c, &[0b01001_011_11100_101]);
    assert!(c.psw.overflow, "divide by zero sets overflow (§4.10)");
}

// ---------- data movement (§4.11-4.20, 4.25-4.27) ----------

#[test]
fn xul_exchanges_upper_and_lower() {
    // §4.11: R1 bits 0-15 <-> R2 bits 16-31.
    let mut c = cpu8k();
    c.set_r(1, 0xAAAA_1111);
    c.set_r(2, 0x2222_BBBB);
    exec1(&mut c, &[0b00000_001_11101_010]); // XUL 1,2
    assert_eq!(c.r(1), 0xBBBB_1111);
    assert_eq!(c.r(2), 0x2222_AAAA);
    // same register: swap halves
    let mut c = cpu8k();
    c.set_r(4, 0x1234_5678);
    exec1(&mut c, &[0b00000_100_11101_100]);
    assert_eq!(c.r(4), 0x5678_1234);
}

#[test]
fn ial_inserts_address_low() {
    // §4.12: the 16-bit EA itself replaces R1 bits 16-31.
    let mut c = cpu8k();
    c.set_r(1, 0x0200_0000); // base 0x200
    c.set_r(2, 0xDEAD_0000);
    // IAL 2,5(1): op 11100, SRS, halfword scaling
    exec1(&mut c, &[0b11100_010_000101_01]);
    assert_eq!(c.r(2), 0xDEAD_0205);
}

#[test]
fn ihl_inserts_halfword_low() {
    let mut c = cpu8k();
    c.mem.write_h(0x340, 0xBEEF).unwrap();
    c.set_r(2, 0x1234_0000);
    // IHL: RS-alternate of op 10000
    exec1(&mut c, &[0b10000_010_11111_011, 0x0340]);
    assert_eq!(c.r(2), 0x1234_BEEF);
}

#[test]
fn l_and_lr_set_cc() {
    // §4.14: load sets the CC from the operand (unlike System/360 L).
    let mut c = cpu8k();
    c.set_r(2, 0x8000_0000);
    exec1(&mut c, &[0b00011_001_11100_010]); // LR 1,2
    assert_eq!(c.r(1), 0x8000_0000);
    assert_eq!(c.psw.cc, CC_NEG);
    let mut c = cpu8k();
    c.mem.write_f(0x480, 0).unwrap();
    exec1(&mut c, &[0b00011_001_11110_011, 0x0480]); // L 1,=0x480
    assert_eq!(c.psw.cc, CC_ZERO);
}

#[test]
fn lh_develops_and_sets_cc() {
    let mut c = cpu8k();
    c.set_r(0, 0x0300u32 << 16);
    c.mem.write_h(0x305, 0x8001).unwrap();
    exec1(&mut c, &[0b10011_001_000101_00]); // LH 1,5(0)
    assert_eq!(c.r(1), 0x8001_0000);
    assert_eq!(c.psw.cc, CC_NEG);
}

#[test]
fn la_loads_address_high() {
    // §4.15: EA to bits 0-15, bits 16-31 zeroed; CC unchanged; with B2=11
    // and AM=0 this is LOAD HALFWORD IMMEDIATE.
    let mut c = cpu8k();
    c.psw.cc = CC_NEG;
    c.set_r(1, 0x0200_0000);
    exec1(&mut c, &[0b11101_010_000111_01]); // LA 2,7(1)
    assert_eq!(c.r(2), 0x0207_0000);
    assert_eq!(c.psw.cc, CC_NEG, "CC not changed");
    // LHI: LA RS-extended with B2=11
    let mut c = cpu8k();
    exec1(&mut c, &[0b11101_011_11110_011, 0x1234]);
    assert_eq!(c.r(3), 0x1234_0000);
}

#[test]
fn lcr_complement() {
    // §4.16: twos complement; carry set only when the operand is zero;
    // overflow when complementing the maximum negative number.
    let mut c = cpu8k();
    c.set_r(2, 5);
    exec1(&mut c, &[0b11101_001_11101_010]); // LCR 1,2
    assert_eq!(c.r(1), 0xFFFF_FFFB);
    assert_eq!(c.psw.cc, CC_NEG);
    assert!(!c.psw.carry && !c.psw.overflow);
    let mut c = cpu8k();
    c.set_r(2, 0);
    exec1(&mut c, &[0b11101_001_11101_010]);
    assert_eq!(c.r(1), 0);
    assert_eq!(c.psw.cc, CC_ZERO);
    assert!(c.psw.carry, "carry set only for zero operand");
    let mut c = cpu8k();
    c.set_r(2, 0x8000_0000);
    exec1(&mut c, &[0b11101_001_11101_010]);
    assert!(c.psw.overflow);
}

#[test]
fn lfxi_immediate_values() {
    // §4.17: value codes 0-15 select -2..13, loaded into bits 0-15 with
    // bits 16-31 zeroed.
    let mut c = cpu8k();
    exec1(&mut c, &[0b10111_011_1110_0000]); // LFXI 3,(code 0 = -2)
    assert_eq!(c.r(3), 0xFFFE_0000);
    let mut c = cpu8k();
    exec1(&mut c, &[0b10111_011_1110_1111]); // code 15 = 13
    assert_eq!(c.r(3), 0x000D_0000);
    let mut c = cpu8k();
    c.set_r(3, 0x1234_5678);
    exec1(&mut c, &[0b10111_011_1110_0010]); // code 2 = 0
    assert_eq!(c.r(3), 0);
}

#[test]
fn lm_stm_round_trip() {
    // §4.19/4.27: all eight registers to/from eight fullwords, ascending.
    let mut c = cpu8k();
    for n in 0..8u8 {
        c.set_r(n, 0x1111_1111u32.wrapping_mul(n as u32));
    }
    // STM =0x400: op 11001, OPX=000, RS-alternate
    exec1(&mut c, &[0b11001_000_11111_011, 0x0400]);
    for n in 0..8u32 {
        assert_eq!(c.mem.read_f(0x400 + 2 * n).unwrap(), 0x1111_1111u32.wrapping_mul(n));
    }
    for n in 0..8u8 {
        c.set_r(n, 0);
    }
    // LM =0x400: OPX=100
    run_at(&mut c, 0x200, &[0b11001_100_11111_011, 0x0400], 1);
    for n in 0..8u8 {
        assert_eq!(c.r(n), 0x1111_1111u32.wrapping_mul(n as u32));
    }
}

#[test]
fn msth_modifies_storage_halfword() {
    // §4.20: immediate added to the halfword operand; CC set; indicators
    // unchanged.
    let mut c = cpu8k();
    c.set_r(1, 0x0400u32 << 16);
    c.mem.write_h(0x403, 5).unwrap();
    exec1(&mut c, &[0b10110_000_000011_01, 0xFFFB]); // MSTH 3(1),-5
    assert_eq!(c.mem.read_h(0x403).unwrap(), 0);
    assert_eq!(c.psw.cc, CC_ZERO);
    assert!(!c.psw.overflow && !c.psw.carry);
}

#[test]
fn st_and_sth() {
    let mut c = cpu8k();
    c.set_r(5, 0xCAFE_F00D);
    exec1(&mut c, &[0b00110_101_11110_011, 0x0410]); // ST 5,=0x410
    assert_eq!(c.mem.read_f(0x410).unwrap(), 0xCAFE_F00D);
    // STH stores bits 0-15 (§4.26)
    run_at(&mut c, 0x200, &[0b10111_101_11110_011, 0x0413], 1); // STH 5,=0x413
    assert_eq!(c.mem.read_h(0x413).unwrap(), 0xCAFE);
    // CC untouched by stores
    assert_eq!(c.psw.cc, CC_ZERO);
}

#[test]
fn td_tally_down() {
    // §4.31: halfword storage operand decremented by one; CC set.
    let mut c = cpu8k();
    c.set_r(1, 0x0400u32 << 16);
    c.mem.write_h(0x404, 1).unwrap();
    // TD 4(1): op 10100 OPX=000
    exec1(&mut c, &[0b10100_000_000100_01]);
    assert_eq!(c.mem.read_h(0x404).unwrap(), 0);
    assert_eq!(c.psw.cc, CC_ZERO);
    run_at(&mut c, 0x200, &[0b10100_000_000100_01], 1);
    assert_eq!(c.mem.read_h(0x404).unwrap(), 0xFFFF);
    assert_eq!(c.psw.cc, CC_NEG);
}

// ---------- traps ----------

#[test]
fn fixed_point_overflow_interrupt_when_unmasked() {
    use lazarus_ap::Trap;
    let mut c = cpu8k();
    c.psw.fixed_overflow_mask = true; // PSW bit 20 = 1: interrupt allowed
    c.set_r(1, 0x7FFF_FFFF);
    c.set_r(2, 1);
    let t = exec1_err(&mut c, &[0b00000_001_11100_010]);
    assert!(matches!(t, Trap::FixedPointOverflow { .. }));
    assert!(c.psw.overflow);
}

#[test]
fn unimplemented_and_illegal_trap() {
    use lazarus_ap::Trap;
    // AER (floating point) decodes but traps as unimplemented.
    let mut c = cpu8k();
    let t = exec1_err(&mut c, &[0b01010_001_11100_010]);
    assert!(matches!(t, Trap::Unimplemented { mnemonic: "AER", .. }));
    // ST has no RR form: illegal.
    let mut c = cpu8k();
    let t = exec1_err(&mut c, &[0b00110_001_11100_010]);
    assert!(matches!(t, Trap::IllegalInstruction { .. }));
}
