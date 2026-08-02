//! Shared helpers for the instruction-level test suite.
//!
//! Tests hand-encode instructions as binary literals (proving the encoding,
//! field layout per IBM 85-C67-001 §2.2.3-2.2.8 and §13) or assemble small
//! programs, then assert on register/memory/condition-code effects.

use lazarus_ap::{asm, Cpu, Memory, Trap};

/// CPU with a small (8K halfword) memory — addresses stay in sector 0.
pub fn cpu8k() -> Cpu {
    Cpu::new(Memory::new(0x2000))
}

/// CPU with the full 19-bit address space (for expanded-addressing tests).
pub fn cpu_full() -> Cpu {
    Cpu::new(Memory::full())
}

/// Load `words` at halfword address `at`, set the IC there, and execute
/// `n` instructions. Panics on any trap.
pub fn run_at(cpu: &mut Cpu, at: u16, words: &[u16], n: usize) {
    try_run_at(cpu, at, words, n).unwrap();
}

pub fn try_run_at(cpu: &mut Cpu, at: u16, words: &[u16], n: usize) -> Result<(), Trap> {
    cpu.mem.load_halfwords(at as u32, words).unwrap();
    cpu.psw.ic = at;
    for _ in 0..n {
        cpu.step()?;
    }
    Ok(())
}

/// Execute a single hand-encoded instruction at address 0x100.
pub fn exec1(cpu: &mut Cpu, words: &[u16]) {
    run_at(cpu, 0x100, words, 1);
}

pub fn exec1_err(cpu: &mut Cpu, words: &[u16]) -> Trap {
    try_run_at(cpu, 0x100, words, 1).unwrap_err()
}

/// Assemble and load a program; IC set to its entry point.
pub fn load_asm(src: &str) -> Cpu {
    let mut cpu = cpu8k();
    let prog = asm::assemble(src).unwrap_or_else(|e| panic!("asm: {e}"));
    prog.load(&mut cpu.mem).unwrap();
    cpu.psw.ic = prog.entry;
    cpu
}

/// Condition-code constants, named as the manual describes the states.
pub const CC_ZERO: u8 = 0b00;
pub const CC_POS: u8 = 0b01;
pub const CC_NEG: u8 = 0b11;
