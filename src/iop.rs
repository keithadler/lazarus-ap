//! Input/Output Processor — structural model (phase 3, in progress).
//!
//! Sources: the AP-101S scan's Appendix I (PCI/PCO Principles of
//! Operation), Appendix II (Master Sequence Controller PoO), and Appendix
//! III (Bus Control Element PoO); see docs/IOP_STATUS.md for what is
//! implemented versus staged.
//!
//! This module currently provides:
//! - The IOP register/state model: the MSC (32-bit ACC, 18-bit X and PC,
//!   busy/wait — App. II §1.1-1.2) and 24 BCEs (program counter, base
//!   register, busy/wait, indicator — App. III §1.2), interrupt
//!   registers, and MIA/discrete enables.
//! - The PCI/PCO command interface (App. I): command word = bit 0
//!   PCO(1)/PCI(0), bits 1-5 one-hot subsystem select (CM/RM/DF/LS/CC),
//!   bit 6 handshake, bits 7-16 data select. The command subset below is
//!   implemented against the App. I summary tables (pages I-4/I-5);
//!   unimplemented commands time out (CC 01 at the CPU), a documented
//!   emulator convention.
//!
//! MSC and BCE *program execution* (their instruction sets, App. II §3 /
//! App. III §3) is the next increment; their encodings are catalogued in
//! docs/IOP_STATUS.md but nothing executes yet — `step` is a stub that
//! never fabricates instruction behavior.

use crate::cpu::{IoSubsystem, PcResponse};

/// Master Sequence Controller register state (App. II §1.2): a 32-bit
/// accumulator, an 18-bit index register, and an 18-bit program counter
/// (bits 0-16 fullword address, bit 17 halfword selector).
#[derive(Debug, Default, Clone)]
pub struct Msc {
    pub acc: u32,
    pub x: u32,
    pub pc: u32,
    /// IOP Busy/Wait register bit 0 (App. II §1.2.2).
    pub busy: bool,
}

/// Bus Control Element register state (App. III §1.2). One per serial
/// data bus; the Shuttle IOP carries 24.
#[derive(Debug, Default, Clone)]
pub struct Bce {
    pub pc: u32,
    pub base: u32,
    pub busy: bool,
    /// BCE indicator bit (set/reset by BCE #SIB/#RIB and MSC @RBI).
    pub indicator: bool,
}

pub const NUM_BCES: usize = 24;

/// Subsystem select field, bits 1-5 of the command word (App. I p. I-2).
/// One-hot; multiple bits set is a hardware misuse the manual warns
/// against, treated here as unrecognized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subsystem {
    ControlMonitor,
    RedundancyManagement,
    DataFlow,
    LocalStore,
    ChannelControl,
}

fn subsystem(cw: u32) -> Option<Subsystem> {
    match (cw >> 26) & 0x1F {
        0b00001 => Some(Subsystem::ControlMonitor),
        0b00010 => Some(Subsystem::RedundancyManagement),
        0b00100 => Some(Subsystem::DataFlow),
        0b01000 => Some(Subsystem::LocalStore),
        0b10000 => Some(Subsystem::ChannelControl),
        _ => None,
    }
}

pub struct Iop {
    pub msc: Msc,
    pub bces: Vec<Bce>,
    /// Interrupt registers A-E (App. I "READ INTERRUPT REG." commands).
    pub interrupt_regs: [u32; 5],
    /// MIA transmitter/receiver enables (App. I p. I-4).
    pub mia_xmtr_enabled: bool,
    pub mia_rcvr_enabled: bool,
    /// Discrete output register.
    pub discrete_out: bool,
    /// PROCESSOR HALT / PROCESSOR ENABLE state (App. I p. I-4): halted
    /// processors do not sequence.
    pub halted: bool,
    /// ENABLE INTERRUPTS state.
    pub interrupts_enabled: bool,
}

impl Default for Iop {
    fn default() -> Iop {
        Iop {
            msc: Msc::default(),
            bces: vec![Bce::default(); NUM_BCES],
            interrupt_regs: [0; 5],
            mia_xmtr_enabled: false,
            mia_rcvr_enabled: false,
            discrete_out: false,
            halted: true,
            interrupts_enabled: false,
        }
    }
}

impl Iop {
    pub fn new() -> Iop {
        Iop::default()
    }

    /// IOP Busy/Wait register (App. II §1.2.2: MSC is bit 0; App. III:
    /// BCE n is bit n+1). Read by PCI "READ STATUS 4 (B/W)".
    pub fn busy_wait_register(&self) -> u32 {
        let mut v = 0u32;
        if self.msc.busy {
            v |= 1 << 31; // bit 0 in IBM numbering
        }
        for (n, bce) in self.bces.iter().enumerate() {
            if bce.busy {
                v |= 1 << (30 - n); // bits 1-24
            }
        }
        v
    }

    /// Advance the IOP. MSC/BCE program execution is not implemented yet
    /// (next increment); this never fabricates instruction behavior.
    pub fn step(&mut self, _mem: &mut crate::mem::Memory) {
        // Intentionally empty until the MSC engine lands.
    }
}

impl IoSubsystem for Iop {
    /// Program-controlled command decode per the App. I summary tables
    /// (hex values from pages I-4/I-5). Data-select is bits 7-16; the hex
    /// summaries below are the full 32-bit command words.
    fn pc(&mut self, cw: u32, data: Option<u32>) -> PcResponse {
        if subsystem(cw).is_none() {
            return PcResponse::Timeout;
        }
        // Mask off the ignored bits (17-31) for command matching; the
        // App. I summaries list bit 17 set for a few test commands, which
        // are unimplemented here anyway.
        let cmd = cw & 0xFFFF_8000;
        match (data.is_some(), cmd) {
            // ---- PCO (outputs) ----
            (true, 0x8508_0000) => {
                self.mia_rcvr_enabled = true;
                PcResponse::OutputAccepted
            }
            (true, 0x8408_0000) => {
                self.mia_rcvr_enabled = false;
                PcResponse::OutputAccepted
            }
            (true, 0x8504_0000) => {
                self.mia_xmtr_enabled = true;
                PcResponse::OutputAccepted
            }
            (true, 0x8404_0000) => {
                self.mia_xmtr_enabled = false;
                PcResponse::OutputAccepted
            }
            (true, 0x8510_0000) => {
                self.discrete_out = true;
                PcResponse::OutputAccepted
            }
            (true, 0x8410_0000) => {
                self.discrete_out = false;
                PcResponse::OutputAccepted
            }
            (true, 0x8620_0000) => {
                self.halted = true;
                PcResponse::OutputAccepted
            }
            (true, 0x8720_0000) => {
                self.halted = false;
                PcResponse::OutputAccepted
            }
            (true, 0x8440_0000) => {
                // MASTER RESET
                *self = Iop::default();
                PcResponse::OutputAccepted
            }
            (true, 0x8814_0000) => {
                self.interrupts_enabled = true;
                PcResponse::OutputAccepted
            }
            (true, 0x9204_0000) => {
                // LOAD MSC BUSY: data word bit 0 (IBM numbering).
                self.msc.busy = data.unwrap_or(0) & 0x8000_0000 != 0;
                PcResponse::OutputAccepted
            }
            // ---- PCI (inputs) ----
            (false, 0x040C_0000) => {
                PcResponse::Input(if self.halted { 0x8000_0000 } else { 0 })
            }
            (false, 0x0408_0000) => {
                PcResponse::Input(if self.discrete_out { 0x8000_0000 } else { 0 })
            }
            (false, 0x1004_0000) => PcResponse::Input(self.busy_wait_register()),
            (false, c @ (0x0800_0000 | 0x0804_0000 | 0x0808_0000 | 0x080C_0000
            | 0x0810_0000)) => {
                let idx = ((c >> 18) & 0x7) as usize; // data-select low bits
                PcResponse::Input(self.interrupt_regs[idx.min(4)])
            }
            // Anything else: not yet implemented — the handshake never
            // completes (documented convention; the CPU sees CC 01).
            _ => PcResponse::Timeout,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_word_subsystem_field() {
        // App. I p. I-2: one-hot subsystem select in bits 1-5.
        assert_eq!(subsystem(0x8720_0000), Some(Subsystem::ControlMonitor));
        assert_eq!(subsystem(0x1004_0000), Some(Subsystem::DataFlow));
        assert_eq!(subsystem(0x0800_0000), Some(Subsystem::RedundancyManagement));
        assert_eq!(subsystem(0x2000_0000), Some(Subsystem::LocalStore));
        assert_eq!(subsystem(0x4000_0000), Some(Subsystem::ChannelControl));
        assert_eq!(subsystem(0x0000_0000), None);
    }

    #[test]
    fn processor_halt_enable_round_trip() {
        let mut iop = Iop::new();
        assert!(iop.halted);
        // PROCESSOR ENABLE (PCO 8720 0000)
        assert_eq!(iop.pc(0x8720_0000, Some(0)), PcResponse::OutputAccepted);
        assert!(!iop.halted);
        // PROCESSOR HALT STATUS (PCI 040C 0000)
        assert_eq!(iop.pc(0x040C_0000, None), PcResponse::Input(0));
        // PROCESSOR HALT (PCO 8620 0000)
        assert_eq!(iop.pc(0x8620_0000, Some(0)), PcResponse::OutputAccepted);
        assert_eq!(iop.pc(0x040C_0000, None), PcResponse::Input(0x8000_0000));
    }

    #[test]
    fn busy_wait_register_layout() {
        let mut iop = Iop::new();
        iop.msc.busy = true;
        iop.bces[0].busy = true;
        iop.bces[23].busy = true;
        // MSC = bit 0, BCE n = bit n+1 (IBM numbering from the MSB).
        let v = iop.busy_wait_register();
        assert_eq!(v & 0x8000_0000, 0x8000_0000);
        assert_eq!(v & 0x4000_0000, 0x4000_0000);
        assert_eq!(v & (1 << (30 - 23)), 1 << (30 - 23));
        assert_eq!(iop.pc(0x1004_0000, None), PcResponse::Input(v));
    }

    #[test]
    fn unimplemented_commands_time_out() {
        let mut iop = Iop::new();
        // FORCE OCTAL MIA BAD PARITY (C180 0000): recognized subsystem,
        // unimplemented command — handshake never completes.
        assert_eq!(iop.pc(0xC180_0000, Some(0)), PcResponse::Timeout);
    }
}
