//! Instruction-level tests: shifts (§6) and logical operations (§7).

mod common;
use common::*;

// ---------- shifts (§6.2-6.9) ----------

#[test]
fn sll_shift_and_carry() {
    // §6.2: zeros enter low-order bits; the carry holds the last bit
    // shifted out of the high-order position.
    let mut c = cpu8k();
    c.set_r(1, 0x8000_0001);
    exec1(&mut c, &[0b11110_001_000001_00]); // SLL 1,1
    assert_eq!(c.r(1), 0x0000_0002);
    assert!(c.psw.carry, "bit shifted out was 1");
    let mut c = cpu8k();
    c.set_r(1, 0x4000_0000);
    exec1(&mut c, &[0b11110_001_000010_00]); // SLL 1,2: out bits 0 then 1
    assert_eq!(c.r(1), 0);
    assert!(c.psw.carry, "last bit out was the 1");
    // count 0: no operation (Figure 6-1), carry untouched
    let mut c = cpu8k();
    c.psw.carry = true;
    c.set_r(1, 0xFFFF_FFFF);
    exec1(&mut c, &[0b11110_001_000000_00]);
    assert_eq!(c.r(1), 0xFFFF_FFFF);
    assert!(c.psw.carry);
}

#[test]
fn sll_computed_count() {
    // Count field 56-63 selects bits 10-15 of GR0-GR7 (Figure 6-1).
    let mut c = cpu8k();
    c.set_r(3, 4u32 << 16); // bits 10-15 of R3 = 4
    c.set_r(1, 1);
    exec1(&mut c, &[0b11110_001_111011_00]); // SLL 1,(count field 59 = GR3)
    assert_eq!(c.r(1), 16);
}

#[test]
fn sra_sign_fill() {
    // §6.4: bits equal to the sign enter vacated high-order positions.
    let mut c = cpu8k();
    c.set_r(1, 0x8000_0000);
    exec1(&mut c, &[0b11110_001_000100_01]); // SRA 1,4
    assert_eq!(c.r(1), 0xF800_0000);
    // carry/overflow not changed (§6.4)
    assert!(!c.psw.carry && !c.psw.overflow);
}

#[test]
fn srl_zero_fill() {
    let mut c = cpu8k();
    c.set_r(1, 0x8000_0000);
    exec1(&mut c, &[0b11110_001_000100_10]); // SRL 1,4
    assert_eq!(c.r(1), 0x0800_0000);
}

#[test]
fn srr_rotate() {
    // §6.8: circular; no bits lost.
    let mut c = cpu8k();
    c.set_r(1, 0x0000_0001);
    exec1(&mut c, &[0b11110_001_000001_11]); // SRR 1,1
    assert_eq!(c.r(1), 0x8000_0000);
}

#[test]
fn double_shifts_use_odd_even_pair() {
    // §6.3/6.5/6.6/6.9: R1 and (R1+1) mod 8 form a 64-bit register.
    // SLDL 2,8
    let mut c = cpu8k();
    c.set_r(2, 0x0012_3456);
    c.set_r(3, 0x789A_BCDE);
    exec1(&mut c, &[0b11111_010_001000_00]);
    assert_eq!(c.r(2), 0x1234_5678);
    assert_eq!(c.r(3), 0x9ABC_DE00);
    // SRDL 2,8
    let mut c = cpu8k();
    c.set_r(2, 0x1234_5678);
    c.set_r(3, 0x9ABC_DE00);
    exec1(&mut c, &[0b11111_010_001000_10]);
    assert_eq!(c.r(2), 0x0012_3456);
    assert_eq!(c.r(3), 0x789A_BCDE);
    // SRDA sign fill
    let mut c = cpu8k();
    c.set_r(2, 0x8000_0000);
    c.set_r(3, 0);
    exec1(&mut c, &[0b11111_010_000100_01]); // SRDA 2,4
    assert_eq!(c.r(2), 0xF800_0000);
    assert_eq!(c.r(3), 0);
    // SRDR by 32 exchanges the registers (§6.9 programming note)
    let mut c = cpu8k();
    c.set_r(2, 0x1111_1111);
    c.set_r(3, 0x2222_2222);
    exec1(&mut c, &[0b11111_010_100000_11]); // SRDR 2,32
    assert_eq!(c.r(2), 0x2222_2222);
    assert_eq!(c.r(3), 0x1111_1111);
    // pair wraps mod 8: R7 pairs with R0 (§2.2.1)
    let mut c = cpu8k();
    c.set_r(7, 0xAAAA_AAAA);
    c.set_r(0, 0x5555_5555);
    exec1(&mut c, &[0b11111_111_100000_11]); // SRDR 7,32
    assert_eq!(c.r(7), 0x5555_5555);
    assert_eq!(c.r(0), 0xAAAA_AAAA);
}

#[test]
fn nct_normalize_and_count() {
    // §6.1: shift R2 left until bit 0 != bit 1, count into R1 bits 0-15.
    let mut c = cpu8k();
    c.set_r(2, 0xFFFF_FFFF);
    exec1(&mut c, &[0b11100_001_11101_010]); // NCT 1,2
    assert_eq!(c.r(2), 0x8000_0000, "all ones normalizes to 80000000");
    assert_eq!(c.r(1), 31u32 << 16, "count 31");
    assert!(c.psw.carry);
    // zero operand: count 0, carry 0
    let mut c = cpu8k();
    c.set_r(1, 0xDEAD_BEEF);
    c.set_r(2, 0);
    exec1(&mut c, &[0b11100_001_11101_010]);
    assert_eq!(c.r(1), 0);
    assert!(!c.psw.carry);
    // already normalized: count 0, carry 1, no shift
    let mut c = cpu8k();
    c.set_r(2, 0x4000_0000);
    exec1(&mut c, &[0b11100_001_11101_010]);
    assert_eq!(c.r(2), 0x4000_0000);
    assert_eq!(c.r(1), 0);
    assert!(c.psw.carry);
}

// ---------- logical operations (§7) ----------

#[test]
fn nr_or_xr_register_forms() {
    // CC: 00 zero, 11 not zero (§7.1).
    let mut c = cpu8k();
    c.set_r(1, 0b1100);
    c.set_r(2, 0b1010);
    exec1(&mut c, &[0b00100_001_11100_010]); // NR 1,2
    assert_eq!(c.r(1), 0b1000);
    assert_eq!(c.psw.cc, CC_NEG);
    let mut c = cpu8k();
    c.set_r(1, 0b1100);
    c.set_r(2, 0b1010);
    exec1(&mut c, &[0b00101_001_11100_010]); // OR 1,2
    assert_eq!(c.r(1), 0b1110);
    let mut c = cpu8k();
    c.set_r(1, 0b1100);
    c.set_r(2, 0b1100);
    exec1(&mut c, &[0b01110_001_11100_010]); // XR 1,2
    assert_eq!(c.r(1), 0);
    assert_eq!(c.psw.cc, CC_ZERO);
}

#[test]
fn n_o_x_storage_forms() {
    let mut c = cpu8k();
    c.set_r(1, 0xFF00_FF00);
    c.mem.write_f(0x400, 0x0F0F_0F0F).unwrap();
    exec1(&mut c, &[0b00100_001_11110_011, 0x0400]); // N 1,=0x400
    assert_eq!(c.r(1), 0x0F00_0F00);
    let mut c = cpu8k();
    c.set_r(1, 0xFF00_FF00);
    c.mem.write_f(0x400, 0x0F0F_0F0F).unwrap();
    exec1(&mut c, &[0b00101_001_11110_011, 0x0400]); // O
    assert_eq!(c.r(1), 0xFF0F_FF0F);
    let mut c = cpu8k();
    c.set_r(1, 0xFF00_FF00);
    c.mem.write_f(0x400, 0x0F0F_0F0F).unwrap();
    exec1(&mut c, &[0b01110_001_11110_011, 0x0400]); // X
    assert_eq!(c.r(1), 0xF00F_F00F);
    assert_eq!(c.psw.cc, CC_NEG);
}

#[test]
fn hi_immediate_forms_operate_on_upper() {
    // §7.2/7.6/7.10: immediate developed with 16 low-order zeros.
    let mut c = cpu8k();
    c.set_r(4, 0xFFFF_1234);
    exec1(&mut c, &[0b10110_110_11100_100, 0x00FF]); // NHI 4,0x00FF
    assert_eq!(c.r(4), 0x00FF_0000, "low half zeroed by developed mask");
    let mut c = cpu8k();
    c.set_r(4, 0x0F00_1234);
    exec1(&mut c, &[0b10110_010_11100_100, 0x00FF]); // OHI 4,0x00FF
    assert_eq!(c.r(4), 0x0FFF_1234);
    let mut c = cpu8k();
    c.set_r(4, 0x0F0F_1234);
    exec1(&mut c, &[0b10110_100_11100_100, 0xFFFF]); // XHI 4,0xFFFF
    assert_eq!(c.r(4), 0xF0F0_1234);
}

#[test]
fn nst_ost_xst_to_storage() {
    let mut c = cpu8k();
    c.set_r(1, 0x0F0F_0F0F);
    c.mem.write_f(0x420, 0xFF00_FF00).unwrap();
    exec1(&mut c, &[0b00100_001_11111_011, 0x0420]); // NST
    assert_eq!(c.mem.read_f(0x420).unwrap(), 0x0F00_0F00);
    assert_eq!(c.r(1), 0x0F0F_0F0F, "register unchanged");
    run_at(&mut c, 0x200, &[0b00101_001_11111_011, 0x0420], 1); // OST
    assert_eq!(c.mem.read_f(0x420).unwrap(), 0x0F0F_0F0F);
    run_at(&mut c, 0x300, &[0b01110_001_11111_011, 0x0420], 1); // XST
    assert_eq!(c.mem.read_f(0x420).unwrap(), 0);
    assert_eq!(c.psw.cc, CC_ZERO);
}

#[test]
fn si_storage_bit_ops() {
    // SB sets, ZB zeroes, NIST ands, XIST xors the halfword operand
    // (§7.13/7.18/7.3/7.7); CC 00 zero / 11 not zero on the result.
    let mut c = cpu8k();
    c.set_r(1, 0x0400u32 << 16);
    c.mem.write_h(0x405, 0b1010).unwrap();
    exec1(&mut c, &[0b10110_010_000101_01, 0b0101]); // SB 5(1),0b0101
    assert_eq!(c.mem.read_h(0x405).unwrap(), 0b1111);
    assert_eq!(c.psw.cc, CC_NEG);
    run_at(&mut c, 0x200, &[0b10110_001_000101_01, 0b1111], 1); // ZB
    assert_eq!(c.mem.read_h(0x405).unwrap(), 0);
    assert_eq!(c.psw.cc, CC_ZERO);
    c.mem.write_h(0x405, 0b1100).unwrap();
    run_at(&mut c, 0x300, &[0b10110_110_000101_01, 0b1010], 1); // NIST
    assert_eq!(c.mem.read_h(0x405).unwrap(), 0b1000);
    run_at(&mut c, 0x400, &[0b10110_100_000101_01, 0b1010], 1); // XIST
    assert_eq!(c.mem.read_h(0x405).unwrap(), 0b0010);
}

#[test]
fn zrb_zeros_register_bits() {
    // §7.19: one bits of the developed immediate zero R2 bits 0-15; bits
    // 16-31 unchanged.
    let mut c = cpu8k();
    c.set_r(3, 0xFFFF_5678);
    exec1(&mut c, &[0b10110_001_11100_011, 0x00FF]); // ZRB 3,0x00FF
    assert_eq!(c.r(3), 0xFF00_5678);
    assert_eq!(c.psw.cc, CC_NEG);
}

#[test]
fn zh_and_shw_do_not_change_cc() {
    // §7.20/7.14: operand set to all zeros / all ones; CC NOT changed.
    let mut c = cpu8k();
    c.psw.cc = CC_POS;
    c.set_r(1, 0x0400u32 << 16);
    c.mem.write_h(0x406, 0x1234).unwrap();
    exec1(&mut c, &[0b10100_001_000110_01]); // ZH 6(1)
    assert_eq!(c.mem.read_h(0x406).unwrap(), 0);
    assert_eq!(c.psw.cc, CC_POS, "ZH leaves CC unchanged");
    run_at(&mut c, 0x200, &[0b10100_010_000110_01], 1); // SHW 6(1)
    assert_eq!(c.mem.read_h(0x406).unwrap(), 0xFFFF);
    assert_eq!(c.psw.cc, CC_POS, "SHW leaves CC unchanged");
}

#[test]
fn tb_th_trb_three_state_cc() {
    // §7.15: 00 selected bits all zero (or mask zero); 11 mixed; 01 all
    // ones. TB 7(1),mask
    let mut c = cpu8k();
    c.set_r(1, 0x0400u32 << 16);
    c.mem.write_h(0x407, 0b1100).unwrap();
    exec1(&mut c, &[0b10110_011_000111_01, 0b0011]);
    assert_eq!(c.psw.cc, CC_ZERO);
    run_at(&mut c, 0x200, &[0b10110_011_000111_01, 0b0110], 1);
    assert_eq!(c.psw.cc, CC_NEG, "mixed");
    run_at(&mut c, 0x300, &[0b10110_011_000111_01, 0b1100], 1);
    assert_eq!(c.psw.cc, CC_POS, "all selected ones");
    run_at(&mut c, 0x400, &[0b10110_011_000111_01, 0], 1);
    assert_eq!(c.psw.cc, CC_ZERO, "mask all zeros");
    // TH: implied all-ones mask (§7.17)
    run_at(&mut c, 0x500, &[0b10100_011_000111_01], 1);
    assert_eq!(c.psw.cc, CC_NEG);
    c.mem.write_h(0x407, 0xFFFF).unwrap();
    run_at(&mut c, 0x600, &[0b10100_011_000111_01], 1);
    assert_eq!(c.psw.cc, CC_POS);
    // TRB: register bits (§7.16)
    let mut c = cpu8k();
    c.set_r(2, 0x00F0_FFFF);
    exec1(&mut c, &[0b10110_011_11100_010, 0x00F0]); // TRB 2,0x00F0
    assert_eq!(c.psw.cc, CC_POS);
}

#[test]
fn sum_search_under_mask() {
    // §7.12: search `count` halfwords for (Ai & M) == (FV & M).
    let mut c = cpu8k();
    // array of 4 halfwords at 0x500, all with low nibble 0xA
    for (i, v) in [0x001A, 0x002A, 0x003A, 0x004A].iter().enumerate() {
        c.mem.write_h(0x500 + i as u32, *v).unwrap();
    }
    c.set_r(1, 0x0500_0001); // address 0x500, modifier +1
    // R1=1, so (R1+1)mod8 = R2 holds M (bits 0-15) and FV (bits 16-31).
    c.set_r(2, (0x000Fu32 << 16) | 0x000A); // M=0x000F, FV=0x000A
    c.set_r(3, 4u32 << 16); // count 4 in R2-of-instruction...
    exec1(&mut c, &[0b10011_001_11101_011]); // SUM 1,3
    assert_eq!(c.psw.cc, CC_ZERO, "all matched");
    assert_eq!(c.r(1) >> 16, 0x0504, "address advanced past the array");
    // mismatch stops with the failing address in R1
    let mut c = cpu8k();
    for (i, v) in [0x001A, 0x002B, 0x003A].iter().enumerate() {
        c.mem.write_h(0x500 + i as u32, *v).unwrap();
    }
    c.set_r(1, 0x0500_0001);
    c.set_r(2, (0x000Fu32 << 16) | 0x000A);
    c.set_r(3, 3u32 << 16);
    exec1(&mut c, &[0b10011_001_11101_011]);
    assert_eq!(c.psw.cc, CC_NEG);
    assert_eq!(c.r(1) >> 16, 0x0501, "address of the mismatch");
}
