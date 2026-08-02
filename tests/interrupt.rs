//! Interrupt-system tests (IBM 85-C67-001 §2.5.2): PSW swaps through the
//! preferred storage area, program-exception codes, SVC, privileged
//! instructions, and the status-switching instructions.

mod common;
use common::*;
use lazarus_ap::{Cpu, Psw};

/// Install a new-PSW doubleword at `addr` with the given IC (all masks
/// off, supervisor state, register set 0).
fn install_psw(c: &mut Cpu, addr: u32, ic: u16) {
    let mut p = Psw::default();
    p.ic = ic;
    c.mem.write_f(addr, p.word0()).unwrap();
    c.mem.write_f(addr + 2, p.word1()).unwrap();
}

#[test]
fn fixed_overflow_swaps_psw() {
    // ENDOP check: overflow indicator + mask -> program interrupt via
    // 0048/004C with code 0004 (Figure 2-20); old PSW holds the state.
    let mut c = cpu8k();
    install_psw(&mut c, 0x4C, 0x0300);
    c.psw.fixed_overflow_mask = true;
    c.set_r(1, 0x7FFF_FFFF);
    c.set_r(2, 1);
    run_at(&mut c, 0x100, &[0b00000_001_11100_010], 1); // AR 1,2
    assert_eq!(c.psw.ic, 0x0300, "executing from the new PSW");
    assert!(!c.psw.overflow, "new PSW's indicators are in effect");
    // old PSW stored at 0x48: IC past AR, CC=11, overflow set, code 0004
    let old0 = c.mem.read_f(0x48).unwrap();
    let old1 = c.mem.read_f(0x4A).unwrap();
    assert_eq!(old0 >> 16, 0x0101);
    assert_eq!((old0 >> 14) & 3, 0b11);
    assert_eq!((old0 >> 12) & 1, 1, "overflow indicator in old PSW");
    assert_eq!(old1 & 0xFFFF, 0x0004, "interrupt code");
}

#[test]
fn illegal_instruction_swaps_with_code_0() {
    let mut c = cpu8k();
    install_psw(&mut c, 0x4C, 0x0300);
    // ST has no RR form -> illegal (§13).
    run_at(&mut c, 0x100, &[0b00110_001_11100_010], 1);
    assert_eq!(c.psw.ic, 0x0300);
    let old0 = c.mem.read_f(0x48).unwrap();
    let old1 = c.mem.read_f(0x4A).unwrap();
    assert_eq!(old0 >> 16, 0x0100, "IC left at the offending instruction");
    assert_eq!(old1 & 0xFFFF, 0x0000);
}

#[test]
fn svc_swap_and_lps_return() {
    // SVC: old PSW to 0058 with the 16-bit EA as interrupt code and the
    // sector extension in bits 40-43; new PSW from 005C (§2.5.2, §9.9).
    // The handler returns with LPS 0058 (§9.3).
    let mut c = cpu8k();
    install_psw(&mut c, 0x5C, 0x0300);
    c.psw.problem_state = true;
    // caller: SVC 0x0123 then (after return) LFXI 1,13
    run_at(
        &mut c,
        0x100,
        &[0b11001_001_11111_011, 0x0123, 0b10111_001_1110_1111],
        1,
    );
    assert_eq!(c.psw.ic, 0x0300);
    assert!(!c.psw.problem_state, "new PSW enters supervisor state");
    let old0 = c.mem.read_f(0x58).unwrap();
    let old1 = c.mem.read_f(0x5A).unwrap();
    assert_eq!(old0 >> 16, 0x0102, "old PSW resumes after the SVC");
    assert_eq!(old1 & 0xFFFF, 0x0123, "SVC code = 16-bit EA");
    assert_eq!((old1 >> 16) & 1, 1, "problem-state bit preserved");
    // handler at 0x300: LPS =0x58 restores the caller's PSW
    c.mem
        .load_halfwords(0x300, &[0b11001_101_11111_011, 0x0058])
        .unwrap();
    c.step().unwrap();
    assert_eq!(c.psw.ic, 0x0102);
    assert!(c.psw.problem_state, "back in problem state");
    // and the caller continues: LFXI 1,13
    c.step().unwrap();
    assert_eq!(c.r(1), 0x000D_0000);
}

#[test]
fn privileged_instruction_in_problem_state() {
    // LPS in problem state: program interrupt code 0001, instruction not
    // executed (§2.3, Figure 2-20).
    let mut c = cpu8k();
    install_psw(&mut c, 0x4C, 0x0300);
    c.psw.problem_state = true;
    run_at(&mut c, 0x100, &[0b11001_101_11111_011, 0x0058], 1); // LPS
    assert_eq!(c.psw.ic, 0x0300);
    assert_eq!(c.mem.read_f(0x4A).unwrap() & 0xFFFF, 0x0001);
    // SSM likewise
    let mut c = cpu8k();
    install_psw(&mut c, 0x4C, 0x0300);
    c.psw.problem_state = true;
    run_at(&mut c, 0x100, &[0b10001_000_11111_011, 0x0400], 1); // SSM
    assert_eq!(c.psw.ic, 0x0300);
    assert_eq!(c.mem.read_f(0x4A).unwrap() & 0xFFFF, 0x0001);
}

#[test]
fn spm_sets_program_mask() {
    // SPM: R2 bits 16-23 -> CC, carry, overflow, and the three arithmetic
    // masks (§9.5).
    let mut c = cpu8k();
    // cc=01, carry=1, overflow=0, fixed mask=0, underflow mask=1, sig=0
    let bits: u32 = (0b01 << 14) | (1 << 13) | (1 << 9);
    c.set_r(2, bits);
    exec1(&mut c, &[0b11001_000_11101_010]); // SPM 2
    assert_eq!(c.psw.cc, CC_POS);
    assert!(c.psw.carry);
    assert!(!c.psw.overflow);
    assert!(c.psw.exp_underflow_mask);
    assert!(!c.psw.significance_mask);
}

#[test]
fn ssm_replaces_system_half_and_wait() {
    use lazarus_ap::Halt;
    // SSM loads PSW bits 32-47; bit 46 is the wait state (§2.5.1.1/9.6).
    let mut c = cpu8k();
    c.mem.write_h(0x400, 0x0002).unwrap(); // wait bit only
    c.mem
        .load_halfwords(0x100, &[0b10001_000_11111_011, 0x0400])
        .unwrap();
    c.psw.ic = 0x100;
    let halt = c.run(10);
    assert!(c.psw.wait);
    assert_eq!(halt, Halt::Wait);
    // register-set select is bit 44: switching isolates the GPR sets
    let mut c = cpu8k();
    c.set_r(1, 111); // set 0
    c.mem.write_h(0x400, 0x0008).unwrap(); // bit 44 = reg set 1
    run_at(&mut c, 0x100, &[0b10001_000_11111_011, 0x0400], 1);
    assert_eq!(c.psw.reg_set, 1);
    assert_eq!(c.r(1), 0, "register set 1 is separate");
    assert_eq!(c.gpr[0][1], 111);
}

#[test]
fn ts_test_and_set() {
    let mut c = cpu8k();
    c.mem.write_h(0x400, 0x0000).unwrap();
    exec1(&mut c, &[0b10111_000_11111_011, 0x0400]); // TS =0x400
    assert_eq!(c.psw.cc, CC_ZERO, "was all zeros");
    assert_eq!(c.mem.read_h(0x400).unwrap(), 0xFFFF, "set to all ones");
    run_at(&mut c, 0x200, &[0b10111_000_11111_011, 0x0400], 1);
    assert_eq!(c.psw.cc, CC_POS, "now all ones: lock was held");
    let mut c = cpu8k();
    c.mem.write_h(0x400, 0x1234).unwrap();
    exec1(&mut c, &[0b10111_000_11111_011, 0x0400]);
    assert_eq!(c.psw.cc, CC_NEG, "mixed");
}

#[test]
fn interrupt_can_switch_register_sets() {
    // §2.5.2.1: "it is possible to switch to the alternate set of general
    // registers when the PSW swap takes place" (new PSW bit 44).
    let mut c = cpu8k();
    let mut handler = Psw::default();
    handler.ic = 0x0300;
    handler.reg_set = 1;
    c.mem.write_f(0x4C, handler.word0()).unwrap();
    c.mem.write_f(0x4E, handler.word1()).unwrap();
    c.psw.fixed_overflow_mask = true;
    c.set_r(1, 0x7FFF_FFFF);
    c.set_r(2, 1);
    run_at(&mut c, 0x100, &[0b00000_001_11100_010], 1);
    assert_eq!(c.psw.reg_set, 1);
    assert_eq!(c.r(1), 0, "handler sees the alternate set");
    assert_eq!(c.gpr[0][1], 0x8000_0000, "interrupted set preserved");
}
