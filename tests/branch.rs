//! Instruction-level tests: branching (IBM 85-C67-001 §5).

mod common;
use common::*;

// ---------- BRANCH ON CONDITION (§5.3-5.6) ----------

#[test]
fn bc_mask_bits() {
    // M1 bit 5 tests CC=00, bit 6 tests CC=11, bit 7 tests CC=01 (§5.3).
    for (cc, m_hit) in [(CC_ZERO, 0b100u16), (CC_NEG, 0b010), (CC_POS, 0b001)] {
        // hit: branch taken
        let mut c = cpu8k();
        c.psw.cc = cc;
        run_at(&mut c, 0x100, &[0b11000_000_11110_011 | m_hit << 8, 0x0500], 1);
        assert_eq!(c.psw.ic, 0x0500, "cc={cc:02b} mask={m_hit:03b} must branch");
        // miss: fall through
        let miss = (!m_hit) & 0b111;
        let mut c = cpu8k();
        c.psw.cc = cc;
        run_at(&mut c, 0x100, &[0b11000_000_11110_011 | miss << 8, 0x0500], 1);
        assert_eq!(c.psw.ic, 0x0102, "cc={cc:02b} mask={miss:03b} must not branch");
    }
    // M1=111 always branches; M1=000 never (§5.3).
    let mut c = cpu8k();
    c.psw.cc = CC_POS;
    run_at(&mut c, 0x100, &[0b11000_111_11110_011, 0x0700], 1);
    assert_eq!(c.psw.ic, 0x0700);
    let mut c = cpu8k();
    run_at(&mut c, 0x100, &[0b11000_000_11110_011, 0x0700], 1);
    assert_eq!(c.psw.ic, 0x0102);
}

#[test]
fn bcr_branches_via_register() {
    // §5.3 RR form: branch address in R2 bits 0-15.
    let mut c = cpu8k();
    c.psw.cc = CC_ZERO;
    c.set_r(2, 0x0640_1111);
    exec1(&mut c, &[0b11000_100_11100_010]); // BCR 4,2
    assert_eq!(c.psw.ic, 0x0640);
}

#[test]
fn bcf_bcb_relative() {
    // BCF adds, BCB subtracts the displacement from the updated IC
    // (§5.6/§5.4).
    let mut c = cpu8k();
    c.psw.cc = CC_ZERO;
    run_at(&mut c, 0x100, &[0b11011_100_000101_00], 1); // BCF 4,5
    assert_eq!(c.psw.ic, 0x0101 + 5);
    let mut c = cpu8k();
    c.psw.cc = CC_ZERO;
    run_at(&mut c, 0x100, &[0b11011_100_000101_10], 1); // BCB 4,5
    assert_eq!(c.psw.ic, 0x0101 - 5);
    // condition false: fall through
    let mut c = cpu8k();
    c.psw.cc = CC_POS;
    run_at(&mut c, 0x100, &[0b11011_100_000101_00], 1);
    assert_eq!(c.psw.ic, 0x0101);
}

#[test]
fn bcre_reloads_sectors() {
    // §5.5: on a taken branch, PSW bits 0-15 and 24-31 (IC, BSR, DSR)
    // come from R2.
    let mut c = cpu_full();
    c.psw.cc = CC_ZERO;
    c.set_r(2, 0x0640_0000 | (0x3 << 4) | 0x5); // ic=0x640, bsr=3, dsr=5
    exec1(&mut c, &[0b11000_100_11101_010]); // BCRE 4,2
    assert_eq!(c.psw.ic, 0x0640);
    assert_eq!(c.psw.bsr, 3);
    assert_eq!(c.psw.dsr, 5);
    // not taken: PSW untouched
    let mut c = cpu_full();
    c.psw.cc = CC_POS;
    c.set_r(2, 0x0640_0035);
    exec1(&mut c, &[0b11000_100_11101_010]);
    assert_eq!(c.psw.ic, 0x0101);
    assert_eq!(c.psw.bsr, 0);
}

// ---------- BRANCH AND LINK (§5.1) ----------

#[test]
fn bal_links_psw_word0() {
    // §5.1: R1 receives PSW bits 0-31 — updated IC, CC, carry, overflow,
    // masks, BSR, DSR; then the branch is taken.
    let mut c = cpu8k();
    c.psw.cc = CC_NEG;
    c.psw.carry = true;
    run_at(&mut c, 0x100, &[0b11100_110_11110_011, 0x0800], 1); // BAL 6,=0x800
    assert_eq!(c.psw.ic, 0x0800);
    let link = c.r(6);
    assert_eq!(link >> 16, 0x0102, "link address = next sequential instr");
    assert_eq!((link >> 14) & 3, 0b11, "CC preserved in link");
    assert_eq!((link >> 13) & 1, 1, "carry preserved in link");
}

#[test]
fn balr_computes_target_before_linking() {
    // §5.1: "First, the branch address is computed", so BALR R,R links
    // and still branches to the register's old contents.
    let mut c = cpu8k();
    c.set_r(6, 0x0900_0000);
    exec1(&mut c, &[0b11100_110_11100_110]); // BALR 6,6
    assert_eq!(c.psw.ic, 0x0900);
    assert_eq!(c.r(6) >> 16, 0x0101);
}

#[test]
fn balr_r2_zero_links_without_branching() {
    // §5.1 programming note: BALR R1,0 takes no branch.
    let mut c = cpu8k();
    exec1(&mut c, &[0b11100_110_11100_000]);
    assert_eq!(c.psw.ic, 0x0101);
    assert_eq!(c.r(6) >> 16, 0x0101);
}

// ---------- BRANCH ON COUNT (§5.7-5.8) ----------

#[test]
fn bct_counts_in_upper_half() {
    // §5.7: bits 0-15 of R1 reduced by one; branch when result non-zero;
    // low-order 16 bits do not participate.
    let mut c = cpu8k();
    c.set_r(1, (3u32 << 16) | 0xFFFF);
    run_at(&mut c, 0x100, &[0b11010_001_11110_011, 0x0300], 1); // BCT 1,=0x300
    assert_eq!(c.r(1), (2u32 << 16) | 0xFFFF);
    assert_eq!(c.psw.ic, 0x0300);
    // count 1 -> 0: no branch
    let mut c = cpu8k();
    c.set_r(1, 1u32 << 16);
    run_at(&mut c, 0x100, &[0b11010_001_11110_011, 0x0300], 1);
    assert_eq!(c.psw.ic, 0x0102);
    // count 0 -> -1: branches (§5.7 programming note)
    let mut c = cpu8k();
    c.set_r(1, 0);
    run_at(&mut c, 0x100, &[0b11010_001_11110_011, 0x0300], 1);
    assert_eq!(c.r(1) >> 16, 0xFFFF);
    assert_eq!(c.psw.ic, 0x0300);
}

#[test]
fn bctr_and_bctb() {
    let mut c = cpu8k();
    c.set_r(1, 5u32 << 16);
    c.set_r(2, 0x0450_0000);
    exec1(&mut c, &[0b11010_001_11100_010]); // BCTR 1,2
    assert_eq!(c.psw.ic, 0x0450);
    // BCTB: branch backward by displacement (§5.8)
    let mut c = cpu8k();
    c.set_r(1, 2u32 << 16);
    run_at(&mut c, 0x100, &[0b11011_001_000100_11], 1); // BCTB 1,4
    assert_eq!(c.psw.ic, 0x0101 - 4);
}

// ---------- BRANCH AND INDEX (§5.2) ----------

#[test]
fn bix_increments_index_decrements_count() {
    // §5.2: R1 = index(0-15) | count(16-31); index += 1, count -= 1;
    // branch when the count prior to update > 0.
    let mut c = cpu8k();
    c.set_r(1, (0x0010u32 << 16) | 2);
    run_at(&mut c, 0x100, &[0b11011_001_11110_011, 0x0333], 1); // BIX 1,=0x333
    assert_eq!(c.r(1), (0x0011u32 << 16) | 1);
    assert_eq!(c.psw.ic, 0x0333);
    // count 0: no branch
    let mut c = cpu8k();
    c.set_r(1, 0x0010u32 << 16);
    run_at(&mut c, 0x100, &[0b11011_001_11110_011, 0x0333], 1);
    assert_eq!(c.r(1), (0x0011u32 << 16) | 0xFFFF);
    assert_eq!(c.psw.ic, 0x0102);
}

// ---------- BRANCH ON OVERFLOW AND CARRY (§5.9-5.10) ----------

#[test]
fn bvc_tests_and_clears_overflow() {
    // M1 bit 6 tests carry, bit 7 tests overflow; the overflow indicator
    // is set to 0 by this instruction; carry unchanged (§5.9).
    let mut c = cpu8k();
    c.psw.overflow = true;
    run_at(&mut c, 0x100, &[0b11001_001_11110_011, 0x0410], 1); // BVC 1 (overflow)
    assert_eq!(c.psw.ic, 0x0410);
    assert!(!c.psw.overflow, "BVC clears the overflow indicator");
    // carry test
    let mut c = cpu8k();
    c.psw.carry = true;
    run_at(&mut c, 0x100, &[0b11001_010_11110_011, 0x0410], 1); // BVC 2 (carry)
    assert_eq!(c.psw.ic, 0x0410);
    assert!(c.psw.carry, "carry unchanged");
    // either
    let mut c = cpu8k();
    c.psw.overflow = true;
    run_at(&mut c, 0x100, &[0b11001_011_11110_011, 0x0410], 1); // BVC 3
    assert_eq!(c.psw.ic, 0x0410);
    // no indicator: fall through
    let mut c = cpu8k();
    run_at(&mut c, 0x100, &[0b11001_011_11110_011, 0x0410], 1);
    assert_eq!(c.psw.ic, 0x0102);
}

#[test]
fn bvc_inverted_tests() {
    // M1 bit 5 = 1 inverts: 100 branches always, 111 branches on no
    // overflow AND no carry (§5.9 table).
    let mut c = cpu8k();
    run_at(&mut c, 0x100, &[0b11001_100_11110_011, 0x0410], 1); // BVC 4
    assert_eq!(c.psw.ic, 0x0410, "M1=100 branches unconditionally");
    let mut c = cpu8k();
    c.psw.carry = true;
    run_at(&mut c, 0x100, &[0b11001_111_11110_011, 0x0410], 1); // BVC 7
    assert_eq!(c.psw.ic, 0x0102, "carry set: no branch for M1=111");
    let mut c = cpu8k();
    run_at(&mut c, 0x100, &[0b11001_111_11110_011, 0x0410], 1);
    assert_eq!(c.psw.ic, 0x0410, "no indicators: M1=111 branches");
}

#[test]
fn bvcr_and_bvcf() {
    let mut c = cpu8k();
    c.psw.overflow = true;
    c.set_r(3, 0x0888_0000);
    exec1(&mut c, &[0b11001_001_11100_011]); // BVCR 1,3
    assert_eq!(c.psw.ic, 0x0888);
    assert!(!c.psw.overflow);
    // BVCF: op 11001 SRS region, bits 14-15 = 01 (§5.10)
    let mut c = cpu8k();
    c.psw.carry = true;
    run_at(&mut c, 0x100, &[0b11001_010_000110_01], 1); // BVCF 2,6
    assert_eq!(c.psw.ic, 0x0101 + 6);
}

// ---------- program flow via the assembler ----------

#[test]
fn loop_program_and_self_loop_halt() {
    use lazarus_ap::Halt;
    // Sum 1..5 by BCT loop; halt by branching to self.
    let mut c = load_asm(
        "
        ORG 0x100
        LFXI 1,5        ; counter in bits 0-15 of R1
        LFXI 2,0        ; sum (as halfword in upper half)
        LOOP: AR 2,1    ; sum += counter (upper halves add)
        BCT 1,LOOP
        DONE: B DONE
        ",
    );
    let halt = c.run(100);
    assert_eq!(halt, Halt::SelfLoop { at: 0x105 });
    // 5+4+3+2+1 = 15 in the upper half of R2
    assert_eq!(c.r(2) >> 16, 15);
}
