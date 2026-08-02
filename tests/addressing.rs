//! Effective-address generation tests (IBM 85-C67-001 §2.2.5-2.2.9, §11.1
//! EA generation summary chart, §14.1 automatic index alignment).

mod common;
use common::*;
use lazarus_ap::Trap;

// L (fullword) and LH (halfword) are the probe instructions throughout.

#[test]
fn srs_base_is_upper_half_and_low_half_ignored() {
    // §2.2.5 Figure 2-7: "the low-order half of the general register
    // containing the base does not participate in SRS addressing".
    let mut c = cpu8k();
    c.set_r(1, (0x0300u32 << 16) | 0xFFFF); // low half must be ignored
    c.mem.write_h(0x305, 0xABCD).unwrap();
    exec1(&mut c, &[0b10011_010_000101_01]); // LH 2,5(1)
    assert_eq!(c.r(2), 0xABCD_0000);
}

#[test]
fn srs_all_four_base_registers() {
    // §2.2.5 Figure 2-6: B2 = 00..11 select GR0..GR3 (GR0 is a valid SRS
    // base, unlike System/360).
    for b2 in 0..4u16 {
        let mut c = cpu8k();
        c.set_r(b2 as u8, 0x0400u32 << 16);
        c.mem.write_h(0x0401, 0x5555).unwrap();
        exec1(&mut c, &[0b10011_001_000001_00 | b2]); // LH 1,1(b2)
        assert_eq!(c.r(1), 0x5555_0000, "B2={b2}");
    }
}

#[test]
fn srs_fullword_scaling() {
    // §2.2.5 Figure 2-8: for fullword operands the displacement aligns to
    // base bit 14, i.e. EA = base + 2*d.
    let mut c = cpu8k();
    c.set_r(1, 0x0400u32 << 16);
    c.mem.write_f(0x400 + 2 * 5, 0x1122_3344).unwrap();
    exec1(&mut c, &[0b00011_010_000101_01]); // L 2,d=5(1)
    assert_eq!(c.r(2), 0x1122_3344);
}

#[test]
fn rs_extended_no_base_when_b2_is_11() {
    // §2.2.8: "When B2 equals 11, base addressing is not performed".
    let mut c = cpu8k();
    c.set_r(3, 0xFFFF_FFFF); // GR3 must NOT contribute
    c.mem.write_f(0x0777, 42).unwrap();
    exec1(&mut c, &[0b00011_001_11110_011, 0x0777]); // L 1,=0x777
    assert_eq!(c.r(1), 42);
}

#[test]
fn rs_extended_with_base() {
    let mut c = cpu8k();
    c.set_r(2, 0x0100u32 << 16);
    c.mem.write_f(0x0100 + 0x0700, 7).unwrap();
    exec1(&mut c, &[0b00011_001_11110_010, 0x0700]); // L 1,0x700(2)
    assert_eq!(c.r(1), 7);
}

#[test]
fn indexed_fullword_alignment_shifts_index() {
    // §14.1: fullword operations shift the index value left one position.
    let mut c = cpu8k();
    c.set_r(2, 0x0400u32 << 16); // base
    c.set_r(5, 3u32 << 16); // index 3 -> 6 halfwords for fullword ops
    c.mem.write_f(0x0400 + 0x10 + 6, 99).unwrap();
    // L 1,0x10(5,2): AM=1, X=5, IA=0, I=0, d11=0x10
    exec1(&mut c, &[0b00011_001_11110_110, (0b101 << 13) | 0x010]);
    assert_eq!(c.r(1), 99);
}

#[test]
fn indexed_halfword_alignment_direct() {
    // §14.1: halfword operations use index bits 0-15 directly.
    let mut c = cpu8k();
    c.set_r(2, 0x0400u32 << 16);
    c.set_r(5, 3u32 << 16);
    c.mem.write_h(0x0400 + 0x10 + 3, 0x7777).unwrap();
    exec1(&mut c, &[0b10011_001_11110_110, (0b101 << 13) | 0x010]); // LH
    assert_eq!(c.r(1), 0x7777_0000);
}

#[test]
fn lm_is_excluded_from_index_alignment() {
    // §4.19/§14.1: LM always has halfword index alignment despite its
    // fullword operands.
    let mut c = cpu8k();
    c.set_r(5, 4u32 << 16); // index 4 stays 4 (not 8)
    for n in 0..8u32 {
        c.mem.write_f(0x0400 + 4 + 2 * n, n + 1).unwrap();
    }
    // LM 0x400(5,3): OPX=100, B2=11 (no base) indexed
    run_at(&mut c, 0x100, &[0b11001_100_11111_111, (0b101 << 13) | 0x400], 1);
    assert_eq!(c.r(0), 1);
    assert_eq!(c.r(7), 8);
}

#[test]
fn ic_relative_forward_and_backward() {
    // §2.2.8 steps 3-4: X=0, IA=0: EA = updated IC ± PEA.
    let mut c = cpu8k();
    // instruction at 0x100, len 2 -> updated IC 0x102; PEA = 0x20
    c.mem.write_f(0x0122, 0x1234_5678).unwrap();
    exec1(&mut c, &[0b00011_001_11110_111, 0x020]); // L 1,ic+0x20 (I=0)
    assert_eq!(c.r(1), 0x1234_5678);
    // backward: I=1
    let mut c = cpu8k();
    c.mem.write_f(0x102 - 0x20, 0x0BAD_F00D).unwrap();
    exec1(&mut c, &[0b00011_001_11110_111, (1 << 11) | 0x020]);
    assert_eq!(c.r(1), 0x0BAD_F00D);
}

#[test]
fn halfword_indirect() {
    // §2.2.8 step 5: X=0, IA=1, I=0: EA = MS(PEA).
    let mut c = cpu8k();
    c.set_r(1, 0x0400u32 << 16);
    c.mem.write_h(0x0410, 0x0555).unwrap(); // pointer at base+0x10
    c.mem.write_f(0x0555, 77).unwrap();
    // L 2,0x10(0,1) with IA=1
    exec1(&mut c, &[0b00011_010_11110_101, (1 << 12) | 0x010]);
    assert_eq!(c.r(2), 77);
}

#[test]
fn indirect_postindexed() {
    // §2.2.8 step 9: X!=0, IA=1, I=0: EA = MS(PEA) + aligned index.
    let mut c = cpu8k();
    c.set_r(1, 0x0400u32 << 16);
    c.set_r(6, 2u32 << 16); // index 2 -> 4 for fullword
    c.mem.write_h(0x0410, 0x0500).unwrap();
    c.mem.write_f(0x0504, 88).unwrap();
    exec1(
        &mut c,
        &[0b00011_010_11110_101, (0b110 << 13) | (1 << 12) | 0x010],
    );
    assert_eq!(c.r(2), 88);
}

#[test]
fn automatic_index_modification() {
    // §2.2.8 step 9 / Figure 2-16: X!=0, IA=0, I=1: after the EA is
    // formed, the index register's modifier (bits 16-31) is added to its
    // address (bits 0-15).
    let mut c = cpu8k();
    c.set_r(1, 0x0400u32 << 16);
    c.set_r(6, (2u32 << 16) | 3); // index 2, modifier +3
    c.mem.write_f(0x0400 + 0x10 + 4, 55).unwrap();
    exec1(
        &mut c,
        &[0b00011_010_11110_101, (0b110 << 13) | (1 << 11) | 0x010],
    );
    assert_eq!(c.r(2), 55, "EA uses the pre-modification index");
    assert_eq!(c.r(6), (5u32 << 16) | 3, "index += modifier afterwards");
}

#[test]
fn fullword_indirect_pointer_is_unimplemented() {
    // §2.2.8 steps 7/10 (fullword indirect address pointer): out of
    // phase-1 scope; must trap, not guess.
    let mut c = cpu8k();
    let t = exec1_err(
        &mut c,
        &[0b00011_010_11110_101, (1 << 12) | (1 << 11) | 0x010],
    );
    assert!(matches!(t, Trap::UnimplementedAddressing { .. }));
}

// ---------- expanded addressing (§2.2.9) ----------

#[test]
fn dsr_expands_data_addresses_with_high_bit_set() {
    // High-order bit 1: the 4-bit DSR replaces it (§2.2.9).
    let mut c = cpu_full();
    c.psw.dsr = 0b0010;
    c.mem.write_f(0x1_0004, 4242).unwrap(); // 0b0010 << 15 | 0x0004
    exec1(&mut c, &[0b00011_001_11110_011, 0x8004]); // L 1,=0x8004
    assert_eq!(c.r(1), 4242);
}

#[test]
fn sector_zero_when_high_bit_clear() {
    // High bit 0 with no base register: implied sector 0000 (§2.2.9).
    let mut c = cpu_full();
    c.psw.dsr = 0b1111; // must not be used
    c.mem.write_f(0x0004, 17).unwrap();
    exec1(&mut c, &[0b00011_001_11110_011, 0x0004]);
    assert_eq!(c.r(1), 17);
}

#[test]
fn dse_expands_based_addresses() {
    // High bit 0 with a base register: that base's DSE supplies the
    // sector (§2.2.9). DSEs default to zero; load one directly here
    // (LXA/LDM are phase-2).
    let mut c = cpu_full();
    c.dse[0][2] = 0b0001;
    c.set_r(2, 0x0100u32 << 16);
    c.mem.write_f(0b0001 << 15 | 0x0110, 31415).unwrap();
    exec1(&mut c, &[0b00011_001_11110_010, 0x0010]); // L 1,0x10(2)
    assert_eq!(c.r(1), 31415);
}

#[test]
fn bsr_expands_branch_addresses() {
    // Branch addresses use the BSR (§2.2.9). BSR=1: branch to 0x8004
    // executes at 0x08004 | (1<<15) = 0x0804 + sector -> 0b0001,0x0004.
    let mut c = cpu_full();
    c.psw.bsr = 0b0001;
    // target instruction: LFXI 1,(code 15 = 13) at 19-bit 0x0800C... use
    // 0x8004 -> expanded (1<<15)|0x0004 = 0x0804*... compute: 0x8004 &
    // 0x7FFF = 0x0004; (1<<15)|0x0004 = 0x8004. Sector 1 IS 0x8000-0xFFFF
    // for BSR=1, so place the target there.
    c.mem.load_halfwords(0x8004, &[0b10111_001_1110_1111]).unwrap();
    run_at(&mut c, 0x100, &[0b11000_111_11110_011, 0x8004], 2); // B =0x8004
    assert_eq!(c.r(1), 0x000D_0000, "executed the instruction in sector 1");
    // with BSR=2 the same 16-bit address reaches a different sector
    let mut c = cpu_full();
    c.psw.bsr = 0b0010;
    c.mem.load_halfwords((0b0010 << 15) | 0x0004, &[0b10111_001_1110_1111]).unwrap();
    run_at(&mut c, 0x100, &[0b11000_111_11110_011, 0x8004], 2);
    assert_eq!(c.r(1), 0x000D_0000);
}

#[test]
fn ic_relative_uses_bsr_not_dsr() {
    // §2.2.8: "IC relative data operand addressing would use BSR instead".
    let mut c = cpu_full();
    c.psw.bsr = 0b0001;
    c.psw.dsr = 0b1111;
    // Run at 16-bit IC 0x8100 (sector 1): physical 0x1_0100.
    let at16: u16 = 0x8100;
    let phys = (1u32 << 15) | 0x0100;
    // L 1,ic+0x20: updated IC = 0x8102, +0x20 = 0x8122 -> BSR sector 1
    c.mem.write_f((1 << 15) | 0x0122, 616).unwrap();
    c.mem
        .load_halfwords(phys, &[0b00011_001_11110_111, 0x020])
        .unwrap();
    c.psw.ic = at16;
    c.step().unwrap();
    assert_eq!(c.r(1), 616);
}
