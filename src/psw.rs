//! Program status word.
//!
//! The 64-bit PSW holds the next instruction address, condition code, carry
//! and overflow indicators, interrupt masks, sector registers, and state
//! bits. Field layout per IBM 85-C67-001 §2.5.1, Figure 2-19:
//!
//! ```text
//! bit  0-15  Next instruction address (16-bit halfword address)
//! bit 16-17  Condition code
//! bit 18     Carry indicator
//! bit 19     Overflow indicator
//! bit 20     Fixed-point arithmetic overflow mask
//! bit 21     Reserved
//! bit 22     Floating-point exponent underflow mask
//! bit 23     Significance mask
//! bit 24-27  Branch sector register (BSR)
//! bit 28-31  Data sector register (DSR)
//! bit 32-39  System mask
//! bit 40-43  Reserved for SVC high-order EA bits
//! bit 44     Register set select (GR set 0 or 1)
//! bit 45     Machine check mask
//! bit 46     Wait state bit
//! bit 47     Problem/supervisor state control bit
//! bit 48-63  Interrupt code / SVC operand PEA
//! ```
//!
//! Condition code values as used throughout the instruction set (e.g.
//! §4.1 ADD "00 zero / 11 negative / 01 positive", §4.5 COMPARE
//! "00 equal / 11 less / 01 greater", §7.1 AND "00 zero / 11 not zero"):
//! the two CC bits form the values 0b00, 0b01, 0b11 (0b10 is used by no
//! fixed-point/logical instruction).

/// Condition code constants (values of the two CC bits).
pub mod cc {
    /// Result zero / operands equal / within limits / test all-zeros.
    pub const ZERO: u8 = 0b00;
    /// Result positive / first operand greater / test all-ones.
    pub const POS: u8 = 0b01;
    /// Result negative / first operand less / logical result not zero /
    /// test mixed.
    pub const NEG: u8 = 0b11;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Psw {
    /// Next instruction address, bits 0-15. Expanded to 19 bits at fetch
    /// time using the BSR (see `Cpu::expand_branch`).
    pub ic: u16,
    /// Condition code, bits 16-17 (two bits).
    pub cc: u8,
    /// Carry indicator, bit 18.
    pub carry: bool,
    /// Overflow indicator, bit 19. Set (sticky) by fixed-point overflow;
    /// cleared only by BVC-family instructions, testing, or PSW load.
    pub overflow: bool,
    /// Fixed-point arithmetic overflow interrupt mask, bit 20
    /// (0 = interrupt inhibited, 1 = allowed).
    pub fixed_overflow_mask: bool,
    /// Floating-point exponent underflow mask, bit 22.
    pub exp_underflow_mask: bool,
    /// Significance mask, bit 23.
    pub significance_mask: bool,
    /// Branch sector register, bits 24-27: replaces the high-order bit of a
    /// branch address when that bit is 1 (§2.2.9 Expanded Addressing).
    pub bsr: u8,
    /// Data sector register, bits 28-31: replaces the high-order bit of a
    /// data address when that bit is 1.
    pub dsr: u8,
    /// System mask, bits 32-39.
    pub sys_mask: u8,
    /// SVC high-order EA bits, bits 40-43.
    pub ea_high: u8,
    /// Register set select, bit 44 (0 = GR set 0, 1 = GR set 1).
    pub reg_set: u8,
    /// Machine check mask, bit 45.
    pub machine_check_mask: bool,
    /// Wait state, bit 46 (false = process, true = wait).
    pub wait: bool,
    /// Problem state, bit 47 (false = supervisor, true = problem).
    pub problem_state: bool,
    /// Interrupt code, bits 48-63.
    pub int_code: u16,
}

impl Default for Psw {
    fn default() -> Psw {
        Psw {
            ic: 0,
            cc: cc::ZERO,
            carry: false,
            overflow: false,
            fixed_overflow_mask: false,
            exp_underflow_mask: false,
            significance_mask: false,
            bsr: 0,
            dsr: 0,
            sys_mask: 0,
            ea_high: 0,
            reg_set: 0,
            machine_check_mask: false,
            wait: false,
            problem_state: false,
            int_code: 0,
        }
    }
}

impl Psw {
    /// PSW bits 0-31 as a fullword — the value BRANCH AND LINK deposits in
    /// its link register (§5.1: "the first word of the current PSW (bits
    /// 0-31) is loaded into general register R1").
    pub fn word0(&self) -> u32 {
        (self.ic as u32) << 16
            | ((self.cc as u32) & 3) << 14
            | (self.carry as u32) << 13
            | (self.overflow as u32) << 12
            | (self.fixed_overflow_mask as u32) << 11
            | (self.exp_underflow_mask as u32) << 9
            | (self.significance_mask as u32) << 8
            | ((self.bsr as u32) & 0xF) << 4
            | (self.dsr as u32) & 0xF
    }

    /// PSW bits 32-63 as a fullword.
    pub fn word1(&self) -> u32 {
        (self.sys_mask as u32) << 24
            | ((self.ea_high as u32) & 0xF) << 20
            | ((self.reg_set as u32) & 1) << 19
            | (self.machine_check_mask as u32) << 18
            | (self.wait as u32) << 17
            | (self.problem_state as u32) << 16
            | self.int_code as u32
    }

    pub fn set_word0(&mut self, w: u32) {
        self.ic = (w >> 16) as u16;
        self.cc = ((w >> 14) & 3) as u8;
        self.carry = w & (1 << 13) != 0;
        self.overflow = w & (1 << 12) != 0;
        self.fixed_overflow_mask = w & (1 << 11) != 0;
        self.exp_underflow_mask = w & (1 << 9) != 0;
        self.significance_mask = w & (1 << 8) != 0;
        self.bsr = ((w >> 4) & 0xF) as u8;
        self.dsr = (w & 0xF) as u8;
    }

    pub fn set_word1(&mut self, w: u32) {
        self.sys_mask = (w >> 24) as u8;
        self.ea_high = ((w >> 20) & 0xF) as u8;
        self.reg_set = ((w >> 19) & 1) as u8;
        self.machine_check_mask = w & (1 << 18) != 0;
        self.wait = w & (1 << 17) != 0;
        self.problem_state = w & (1 << 16) != 0;
        self.int_code = w as u16;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word0_round_trip() {
        let mut p = Psw::default();
        p.ic = 0x1234;
        p.cc = cc::NEG;
        p.carry = true;
        p.overflow = false;
        p.bsr = 0xA;
        p.dsr = 0x5;
        let w = p.word0();
        // ic in bits 0-15, cc in 16-17, carry 18, bsr 24-27, dsr 28-31
        assert_eq!(w >> 16, 0x1234);
        assert_eq!((w >> 14) & 3, 0b11);
        assert_eq!((w >> 13) & 1, 1);
        assert_eq!((w >> 12) & 1, 0);
        assert_eq!((w >> 4) & 0xF, 0xA);
        assert_eq!(w & 0xF, 0x5);
        let mut q = Psw::default();
        q.set_word0(w);
        assert_eq!(q.ic, p.ic);
        assert_eq!(q.cc, p.cc);
        assert_eq!(q.carry, p.carry);
        assert_eq!(q.bsr, p.bsr);
        assert_eq!(q.dsr, p.dsr);
    }
}
