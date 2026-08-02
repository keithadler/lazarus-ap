//! AP-101S CPU: machine state, effective-address generation, and execution
//! of the phase-1 instruction subset.
//!
//! All semantics cite IBM 85-C67-001 (see docs/SOURCES.md); section numbers
//! in comments refer to that manual.

use crate::decode::{self, Decoded, Instr, Operand, Width};
use crate::mem::{AddressError, Memory};
use crate::psw::{cc, Psw};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trap {
    /// Storage access outside installed memory.
    Address { addr: u32, at: u32 },
    /// Encoding with no §13 op-code assignment.
    IllegalInstruction { hw1: u16, at: u32 },
    /// Instruction whose encoding is known but whose execution is out of
    /// phase-1 scope (floating point, I/O, privileged/status ops).
    Unimplemented { mnemonic: &'static str, at: u32 },
    /// Addressing mode out of phase-1 scope: the fullword indirect address
    /// pointer modes (RS indexed with I=1 and IA=1, §2.2.8 steps 7/10,
    /// Figure 2-17).
    UnimplementedAddressing { at: u32 },
    /// A fixed-point overflow occurred while the fixed-point overflow mask
    /// (PSW bit 20) allows the interrupt. Phase 1 has no interrupt system,
    /// so the emulator halts here instead of PSW-swapping.
    FixedPointOverflow { at: u32 },
}

/// Why `run` stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Halt {
    /// A taken branch targeted its own instruction address — the idiomatic
    /// "halt" of a machine with no stop instruction in problem state.
    SelfLoop { at: u16 },
    /// PSW wait state (§2.5.1.1 bit 46).
    Wait,
    /// Step budget exhausted.
    StepLimit,
    Trap(Trap),
}

pub struct Cpu {
    pub mem: Memory,
    pub psw: Psw,
    /// Two sets of eight 32-bit fixed-point general registers; the PSW
    /// register-select bit (bit 44) picks the set in current use (§2.2.1).
    pub gpr: [[u32; 8]; 2],
    /// One set of eight 32-bit floating-point registers (§2.2.1). Present
    /// in the machine state; no phase-1 instruction operates on them.
    pub fpr: [u32; 8],
    /// 4-bit Data Sector Extension registers, one per general register per
    /// set (§2.2.1, §2.2.9). Loaded by LXA/LDM (unimplemented); reset value
    /// zero, giving sector-0 behavior.
    pub dse: [[u8; 8]; 2],
    /// Steps executed since construction.
    pub steps: u64,
}

impl Default for Cpu {
    fn default() -> Cpu {
        Cpu::new(Memory::full())
    }
}

/// Second operand resolved either to a register or a 19-bit storage address.
enum Ea {
    Reg(u8),
    /// (16-bit address before expansion, 19-bit expanded address)
    Mem { ea16: u16, addr: u32 },
}

impl Cpu {
    pub fn new(mem: Memory) -> Cpu {
        Cpu {
            mem,
            psw: Psw::default(),
            gpr: [[0; 8]; 2],
            fpr: [0; 8],
            dse: [[0; 8]; 2],
            steps: 0,
        }
    }

    // ----- register access (current set) -----

    pub fn r(&self, n: u8) -> u32 {
        self.gpr[self.psw.reg_set as usize][(n & 7) as usize]
    }

    pub fn set_r(&mut self, n: u8, v: u32) {
        self.gpr[self.psw.reg_set as usize][(n & 7) as usize] = v;
    }

    fn r_upper(&self, n: u8) -> u16 {
        (self.r(n) >> 16) as u16
    }

    fn set_r_upper(&mut self, n: u8, v: u16) {
        self.set_r(n, ((v as u32) << 16) | (self.r(n) & 0xFFFF));
    }

    fn set_r_lower(&mut self, n: u8, v: u16) {
        self.set_r(n, (self.r(n) & 0xFFFF_0000) | v as u32);
    }

    // ----- expanded addressing (§2.2.9, Figure 2-18) -----

    /// Expand a 16-bit data-operand address to 19 bits. When the high-order
    /// bit is 1 the 4-bit DSR replaces it; when 0 and a base register was
    /// used, that base register's DSE replaces it; otherwise an implied
    /// sector of 0000.
    fn expand_data(&self, ea16: u16, base_reg: Option<u8>) -> u32 {
        if ea16 & 0x8000 != 0 {
            ((self.psw.dsr as u32) << 15) | (ea16 & 0x7FFF) as u32
        } else if let Some(b) = base_reg {
            let dse = self.dse[self.psw.reg_set as usize][(b & 7) as usize];
            ((dse as u32) << 15) | ea16 as u32
        } else {
            ea16 as u32
        }
    }

    /// Expand a 16-bit branch (or IC-relative) address to 19 bits using the
    /// BSR (§2.2.9: branch addresses; §2.2.8: IC-relative operands use BSR
    /// in place of DSR).
    pub fn expand_branch(&self, ea16: u16) -> u32 {
        if ea16 & 0x8000 != 0 {
            ((self.psw.bsr as u32) << 15) | (ea16 & 0x7FFF) as u32
        } else {
            ea16 as u32
        }
    }

    fn read_h(&self, addr: u32, at: u32) -> Result<u16, Trap> {
        self.mem.read_h(addr).map_err(|e| trap_addr(e, at))
    }

    // ----- effective address generation -----

    /// Automatic index alignment (§14.1): halfword operations use index
    /// register bits 0-15 directly; fullword operations shift the index
    /// value one position left (bit 0 of the index is lost). LM and STM
    /// always use halfword alignment (§14.1).
    fn aligned_index(&self, x: u8, instr: Instr) -> u16 {
        let idx = self.r_upper(x);
        match instr.width() {
            Width::Half => idx,
            Width::Full if instr.halfword_index_alignment() => idx,
            Width::Full => idx << 1,
        }
    }

    /// Resolve the second operand to a register or a storage address.
    /// `branch` selects BSR-based final expansion (branch targets).
    fn resolve(&mut self, dec: &Decoded, at: u32, branch: bool) -> Result<Ea, Trap> {
        let instr = dec.instr;
        let (ea16, base_reg): (u16, Option<u8>) = match dec.operand {
            Operand::R(r2) => return Ok(Ea::Reg(r2)),
            Operand::None | Operand::Count(_) => {
                unreachable!("no storage operand to resolve")
            }
            // SRS: displacement added to base register bits 0-15; for
            // fullword operands the displacement is aligned one bit left
            // (§2.2.5, Figures 2-7/2-8). B2 = 00..11 selects GR0..GR3, all
            // of which act as bases in SRS (§2.2.5 Figure 2-6).
            Operand::Srs { d, b2 } => {
                let scaled = match instr.width() {
                    Width::Half => d as u16,
                    Width::Full => (d as u16) << 1,
                };
                (self.r_upper(b2).wrapping_add(scaled), Some(b2))
            }
            // RS extended (AM=0): full 16-bit displacement, identical
            // alignment for all operand sizes; B2=11 means no base — the
            // displacement is the address (§2.2.8).
            Operand::RsExt { d16, b2 } => {
                if b2 == 0b11 {
                    (d16, None)
                } else {
                    (self.r_upper(b2).wrapping_add(d16), Some(b2))
                }
            }
            // RS indexed (AM=1): PEA = base (or 0 for B2=11) + 11-bit
            // displacement; then X/IA/I select the mode (§2.2.8 steps 1-10).
            Operand::RsIdx { d11, b2, x, ia, i } => {
                let (base, base_reg) = if b2 == 0b11 {
                    (0u16, None)
                } else {
                    (self.r_upper(b2), Some(b2))
                };
                let pea = base.wrapping_add(d11);
                match (x, ia, i) {
                    // IC-relative: EA = updated IC ± PEA, expanded with the
                    // BSR in place of the DSR (§2.2.8 steps 3-4).
                    (0, false, false) => {
                        let ea16 = self.psw.ic.wrapping_add(pea);
                        return Ok(Ea::Mem { ea16, addr: self.expand_branch(ea16) });
                    }
                    (0, false, true) => {
                        let ea16 = self.psw.ic.wrapping_sub(pea);
                        return Ok(Ea::Mem { ea16, addr: self.expand_branch(ea16) });
                    }
                    // Halfword indirect: EA = MS(PEA); the fetched pointer's
                    // second-stage expansion uses DSR/0000 (no base)
                    // (§2.2.8 step 5, §2.2.9).
                    (0, true, false) => {
                        let ptr_addr = self.expand_data(pea, base_reg);
                        let ptr = self.read_h(ptr_addr, at)?;
                        let addr = if branch {
                            self.expand_branch(ptr)
                        } else {
                            self.expand_data(ptr, None)
                        };
                        return Ok(Ea::Mem { ea16: ptr, addr });
                    }
                    // Indexed: EA = PEA + aligned index (§2.2.8 step 8).
                    (_, false, false) => {
                        (pea.wrapping_add(self.aligned_index(x, instr)), base_reg)
                    }
                    // Indexed with automatic index modification: EA as
                    // above; afterwards the index register's modifier
                    // (bits 16-31) is added to its address (bits 0-15)
                    // (§2.2.8 step 9, Figure 2-16).
                    (_, false, true) if x != 0 => {
                        let ea16 = pea.wrapping_add(self.aligned_index(x, instr));
                        let xv = self.r(x);
                        let modified =
                            ((xv >> 16) as u16).wrapping_add(xv as u16);
                        self.set_r_upper(x, modified);
                        (ea16, base_reg)
                    }
                    // Indirect with postindexing: EA = MS(PEA) + aligned
                    // index; second-stage expansion without base (§2.2.8
                    // step 9 [indexed indirect]).
                    (_, true, false) => {
                        let ptr_addr = self.expand_data(pea, base_reg);
                        let ptr = self.read_h(ptr_addr, at)?;
                        let ea16 = ptr.wrapping_add(self.aligned_index(x, instr));
                        let addr = if branch {
                            self.expand_branch(ea16)
                        } else {
                            self.expand_data(ea16, None)
                        };
                        return Ok(Ea::Mem { ea16, addr });
                    }
                    // Fullword indirect address pointer modes (I=1 with
                    // IA=1, and X=0/IA=0 handled above): §2.2.8 steps 7/10,
                    // Figure 2-17. Not implemented in phase 1.
                    _ => return Err(Trap::UnimplementedAddressing { at }),
                }
            }
        };
        let addr = if branch {
            self.expand_branch(ea16)
        } else {
            self.expand_data(ea16, base_reg)
        };
        Ok(Ea::Mem { ea16, addr })
    }

    /// Fullword second operand (register or storage).
    fn fetch_full(&mut self, dec: &Decoded, at: u32) -> Result<u32, Trap> {
        match self.resolve(dec, at, false)? {
            Ea::Reg(r2) => Ok(self.r(r2)),
            Ea::Mem { addr, .. } => self.mem.read_f(addr).map_err(|e| trap_addr(e, at)),
        }
    }

    /// Halfword storage second operand, developed into a fullword by using
    /// it as the most significant 16 bits with 16 low-order zeros (§4.0).
    fn fetch_half_developed(&mut self, dec: &Decoded, at: u32) -> Result<u32, Trap> {
        match self.resolve(dec, at, false)? {
            Ea::Reg(_) => unreachable!("halfword ops have no RR form"),
            Ea::Mem { addr, .. } => {
                Ok((self.read_h(addr, at)? as u32) << 16)
            }
        }
    }

    fn storage_addr(&mut self, dec: &Decoded, at: u32) -> Result<u32, Trap> {
        match self.resolve(dec, at, false)? {
            Ea::Reg(_) => unreachable!("storage-only operand"),
            Ea::Mem { addr, .. } => Ok(addr),
        }
    }

    /// 16-bit effective address for LA/IAL (no 19-bit expansion, §4.12/4.15)
    /// and for branch targets (stored into the 16-bit PSW IC field).
    fn ea16(&mut self, dec: &Decoded, at: u32, branch: bool) -> Result<u16, Trap> {
        match self.resolve(dec, at, branch)? {
            Ea::Reg(_) => unreachable!("address-only operand"),
            Ea::Mem { ea16, .. } => Ok(ea16),
        }
    }

    // ----- condition code and indicators -----

    /// Arithmetic/load CC: 00 zero, 11 negative, 01 positive (e.g. §4.1).
    fn cc_value32(&mut self, v: u32) {
        self.psw.cc = if v == 0 {
            cc::ZERO
        } else if v & 0x8000_0000 != 0 {
            cc::NEG
        } else {
            cc::POS
        };
    }

    fn cc_value16(&mut self, v: u16) {
        self.psw.cc = if v == 0 {
            cc::ZERO
        } else if v & 0x8000 != 0 {
            cc::NEG
        } else {
            cc::POS
        };
    }

    /// Compare CC: 00 equal, 11 first < second, 01 first > second (§4.5).
    fn cc_compare(&mut self, a: i32, b: i32) {
        self.psw.cc = if a == b {
            cc::ZERO
        } else if a < b {
            cc::NEG
        } else {
            cc::POS
        };
    }

    /// Logical CC: 00 zero, 11 not zero (§7.1).
    fn cc_logical32(&mut self, v: u32) {
        self.psw.cc = if v == 0 { cc::ZERO } else { cc::NEG };
    }

    fn cc_logical16(&mut self, v: u16) {
        self.psw.cc = if v == 0 { cc::ZERO } else { cc::NEG };
    }

    /// Test CC (§7.15 TEST BITS): 00 selected bits all zero (or mask zero),
    /// 11 mixed, 01 all ones.
    fn cc_test(&mut self, value: u32, mask: u32) {
        let sel = value & mask;
        self.psw.cc = if mask == 0 || sel == 0 {
            cc::ZERO
        } else if sel == mask {
            cc::POS
        } else {
            cc::NEG
        };
    }

    /// Add setting carry and (sticky) overflow per §4.1: carry indicates a
    /// carry out of the high-order bit position; overflow is set to one on
    /// signed overflow and "if the overflow indicator already contains a
    /// one, it is not altered". Returns (result, overflowed).
    fn add_flags(&mut self, a: u32, b: u32) -> (u32, bool) {
        let (r, carry) = a.overflowing_add(b);
        let ovf = (!(a ^ b) & (a ^ r)) & 0x8000_0000 != 0;
        self.psw.carry = carry;
        if ovf {
            self.psw.overflow = true;
        }
        (r, ovf)
    }

    /// Subtraction a - b "performed by adding the ones complement of the
    /// second operand and a low-order one"; overflow, carry and CC reflect
    /// that addition (§4.28).
    fn sub_flags(&mut self, a: u32, b: u32) -> (u32, bool) {
        let nb = !b;
        let sum = a as u64 + nb as u64 + 1;
        let r = sum as u32;
        let carry = sum >> 32 != 0;
        let ovf = ((a ^ b) & (a ^ r)) & 0x8000_0000 != 0;
        self.psw.carry = carry;
        if ovf {
            self.psw.overflow = true;
        }
        (r, ovf)
    }

    /// Fixed-point overflow program interrupt: taken only when PSW bit 20
    /// allows it (§2.5.1.1, interrupt table). No interrupt system in phase
    /// 1 — the emulator traps instead.
    fn fixed_overflow(&self, ovf: bool, at: u32) -> Result<(), Trap> {
        if ovf && self.psw.fixed_overflow_mask {
            Err(Trap::FixedPointOverflow { at })
        } else {
            Ok(())
        }
    }

    // ----- fetch/execute -----

    /// Execute one instruction. Returns the 19-bit address it was fetched
    /// from.
    pub fn step(&mut self) -> Result<u32, Trap> {
        let at = self.expand_branch(self.psw.ic);
        let hw1 = self.read_h(at, at)?;
        // The second halfword is fetched only if the format needs it.
        let hw2 = self.mem.read_h(at.wrapping_add(1)).unwrap_or(0);
        let dec = decode::decode(hw1, hw2)
            .map_err(|decode::DecodeError::Illegal { hw1 }| Trap::IllegalInstruction { hw1, at })?;
        if dec.len == 2 {
            // Ensure the second halfword really was addressable.
            self.read_h(at.wrapping_add(1), at)?;
        }
        // The PSW reflects the updated IC during execution: BAL links to
        // the next sequential instruction (§5.1) and IC-relative/BCF/BCB
        // arithmetic uses the updated IC (§2.2.8, §5.4, §5.6).
        self.psw.ic = self.psw.ic.wrapping_add(dec.len as u16);
        self.exec(&dec, at)?;
        self.steps += 1;
        Ok(at)
    }

    /// Run until a halt condition. A taken branch that targets itself
    /// (tight loop) halts — the conventional way for test programs to stop.
    pub fn run(&mut self, max_steps: u64) -> Halt {
        for _ in 0..max_steps {
            if self.psw.wait {
                return Halt::Wait;
            }
            let before = self.psw.ic;
            match self.step() {
                Ok(_) => {
                    if self.psw.ic == before {
                        return Halt::SelfLoop { at: before };
                    }
                }
                Err(t) => return Halt::Trap(t),
            }
        }
        Halt::StepLimit
    }

    fn exec(&mut self, dec: &Decoded, at: u32) -> Result<(), Trap> {
        use Instr::*;
        let r1 = dec.r1;
        match dec.instr {
            NotImplemented(m) => return Err(Trap::Unimplemented { mnemonic: m, at }),

            // ---- fixed point: add/subtract (§4.1-4.4, 4.28-4.30) ----
            A | Ar => {
                let op = self.fetch_full(dec, at)?;
                let (r, ovf) = self.add_flags(self.r(r1), op);
                self.set_r(r1, r);
                self.cc_value32(r);
                self.fixed_overflow(ovf, at)?;
            }
            Ah => {
                let op = self.fetch_half_developed(dec, at)?;
                let (r, ovf) = self.add_flags(self.r(r1), op);
                self.set_r(r1, r);
                self.cc_value32(r);
                self.fixed_overflow(ovf, at)?;
            }
            Ahi => {
                let op = (dec.imm as u32) << 16;
                let (r, ovf) = self.add_flags(self.r(r1), op);
                self.set_r(r1, r);
                self.cc_value32(r);
                self.fixed_overflow(ovf, at)?;
            }
            Ast => {
                // R1 + second operand -> second operand location (§4.4).
                let addr = self.storage_addr(dec, at)?;
                let op = self.mem.read_f(addr).map_err(|e| trap_addr(e, at))?;
                let (r, ovf) = self.add_flags(self.r(r1), op);
                self.mem.write_f(addr, r).map_err(|e| trap_addr(e, at))?;
                self.cc_value32(r);
                self.fixed_overflow(ovf, at)?;
            }
            S | Sr => {
                let op = self.fetch_full(dec, at)?;
                let (r, ovf) = self.sub_flags(self.r(r1), op);
                self.set_r(r1, r);
                self.cc_value32(r);
                self.fixed_overflow(ovf, at)?;
            }
            Sh => {
                let op = self.fetch_half_developed(dec, at)?;
                let (r, ovf) = self.sub_flags(self.r(r1), op);
                self.set_r(r1, r);
                self.cc_value32(r);
                self.fixed_overflow(ovf, at)?;
            }
            Sst => {
                // R1 subtracted FROM the second operand; result to storage
                // (§4.29).
                let addr = self.storage_addr(dec, at)?;
                let op = self.mem.read_f(addr).map_err(|e| trap_addr(e, at))?;
                let (r, ovf) = self.sub_flags(op, self.r(r1));
                self.mem.write_f(addr, r).map_err(|e| trap_addr(e, at))?;
                self.cc_value32(r);
                self.fixed_overflow(ovf, at)?;
            }

            // ---- compares (§4.5-4.9) — indicators not changed ----
            C | Cr => {
                let op = self.fetch_full(dec, at)?;
                self.cc_compare(self.r(r1) as i32, op as i32);
            }
            Ch => {
                let op = self.fetch_half_developed(dec, at)?;
                self.cc_compare(self.r(r1) as i32, op as i32);
            }
            Chi => {
                let op = (dec.imm as u32) << 16;
                self.cc_compare(self.r(r1) as i32, op as i32);
            }
            Cist => {
                // Immediate compared with the halfword storage operand
                // (§4.9): CC 11 = immediate less than storage.
                let addr = self.storage_addr(dec, at)?;
                let v = self.read_h(addr, at)?;
                self.cc_compare(dec.imm as i16 as i32, v as i16 as i32);
            }
            Cbl => {
                // §4.6: R1 bits 0-15 address a 16-bit operand, R2 bits 0-15
                // address an upper/lower limit fullword; bits 16-31 of each
                // are modifiers added to the address halves afterwards
                // (overflow/carry out of the address ignored).
                let r2 = operand_reg(dec);
                let r1v = self.r(r1);
                let r2v = self.r(r2);
                let op_addr = self.expand_data((r1v >> 16) as u16, None);
                let lim_addr = self.expand_data((r2v >> 16) as u16, None);
                let operand = self.read_h(op_addr, at)? as i16;
                let upper = self.read_h(lim_addr, at)? as i16;
                let lower = self.read_h(lim_addr.wrapping_add(1), at)? as i16;
                self.psw.cc = if operand < lower {
                    cc::NEG
                } else if operand > upper {
                    cc::POS
                } else {
                    cc::ZERO
                };
                let m1 = ((r1v >> 16) as u16).wrapping_add(r1v as u16);
                self.set_r(r1, ((m1 as u32) << 16) | (r1v & 0xFFFF));
                let m2 = ((r2v >> 16) as u16).wrapping_add(r2v as u16);
                self.set_r(r2, ((m2 as u32) << 16) | (r2v & 0xFFFF));
            }

            // ---- multiply / divide (§4.10, 4.21-4.24) ----
            M | Mr => {
                let op = self.fetch_full(dec, at)?;
                let ovf = self.multiply_frac32(r1, op);
                self.fixed_overflow(ovf, at)?;
            }
            Mh => {
                let addr = self.storage_addr(dec, at)?;
                let b = self.read_h(addr, at)?;
                let ovf = self.multiply_frac16(r1, b);
                self.fixed_overflow(ovf, at)?;
            }
            Mhi => {
                let ovf = self.multiply_frac16(r1, dec.imm);
                self.fixed_overflow(ovf, at)?;
            }
            Mih => {
                // Integer halfword multiply (§4.24): 16x16 integer product;
                // overflow unless it fits a signed halfword; the halfword
                // product replaces bits 0-15 of R1, bits 16-31 zeroed.
                let addr = self.storage_addr(dec, at)?;
                let b = self.read_h(addr, at)? as i16 as i32;
                let a = (self.r(r1) >> 16) as u16 as i16 as i32;
                let p = a * b;
                let ovf = p < i16::MIN as i32 || p > i16::MAX as i32;
                if ovf {
                    self.psw.overflow = true;
                }
                self.set_r(r1, (p as u16 as u32) << 16);
                self.fixed_overflow(ovf, at)?;
            }
            D | Dr => {
                let op = self.fetch_full(dec, at)?;
                let ovf = self.divide_frac(r1, op as i32);
                self.fixed_overflow(ovf, at)?;
            }

            // ---- data movement (§4.11-4.20, 4.25-4.27) ----
            Xul => {
                // Exchange R1 bits 0-15 with R2 bits 16-31 (§4.11).
                let r2 = operand_reg(dec);
                let r1v = self.r(r1);
                let r2v = self.r(r2);
                if r1 == r2 {
                    self.set_r(r1, (r1v << 16) | (r1v >> 16));
                } else {
                    self.set_r(r1, ((r2v & 0xFFFF) << 16) | (r1v & 0xFFFF));
                    self.set_r(r2, (r2v & 0xFFFF_0000) | (r1v >> 16));
                }
            }
            Ial => {
                let ea = self.ea16(dec, at, false)?;
                self.set_r_lower(r1, ea);
            }
            Ihl => {
                let addr = self.storage_addr(dec, at)?;
                let v = self.read_h(addr, at)?;
                self.set_r_lower(r1, v);
            }
            L | Lr => {
                let op = self.fetch_full(dec, at)?;
                self.set_r(r1, op);
                self.cc_value32(op);
            }
            Lh => {
                let op = self.fetch_half_developed(dec, at)?;
                self.set_r(r1, op);
                self.cc_value32(op);
            }
            La => {
                // 16-bit halfword address into bits 0-15, low bits zeroed;
                // no 19-bit expansion (§4.15). With B2=11/AM=0 this is LOAD
                // HALFWORD IMMEDIATE.
                let ea = self.ea16(dec, at, false)?;
                self.set_r(r1, (ea as u32) << 16);
            }
            Lcr => {
                let r2 = operand_reg(dec);
                let op = self.r(r2);
                let r = op.wrapping_neg();
                self.set_r(r1, r);
                self.cc_value32(r);
                // §4.16: overflow when the maximum negative number is
                // complemented; carry set only when the operand is zero.
                let ovf = op == 0x8000_0000;
                if ovf {
                    self.psw.overflow = true;
                }
                self.psw.carry = op == 0;
                self.fixed_overflow(ovf, at)?;
            }
            Lfxi => {
                // Immediate values -2..13 selected by the 4-bit value code,
                // loaded into bits 0-15 with bits 16-31 zeroed (§4.17).
                let value = dec.imm as i32 - 2;
                self.set_r(r1, (value as i16 as u16 as u32) << 16);
            }
            Lm => {
                // All eight general registers from eight fullwords starting
                // at the second operand address, ascending (§4.19).
                let addr = self.storage_addr(dec, at)?;
                for n in 0..8u8 {
                    let v = self
                        .mem
                        .read_f(addr.wrapping_add(2 * n as u32))
                        .map_err(|e| trap_addr(e, at))?;
                    self.set_r(n, v);
                }
            }
            Stm => {
                let addr = self.storage_addr(dec, at)?;
                for n in 0..8u8 {
                    self.mem
                        .write_f(addr.wrapping_add(2 * n as u32), self.r(n))
                        .map_err(|e| trap_addr(e, at))?;
                }
            }
            Msth => {
                // Immediate (twos complement) added to the halfword storage
                // operand (§4.20).
                let addr = self.storage_addr(dec, at)?;
                let v = self.read_h(addr, at)?;
                let r = v.wrapping_add(dec.imm);
                self.mem.write_h(addr, r).map_err(|e| trap_addr(e, at))?;
                self.cc_value16(r);
            }
            St => {
                let addr = self.storage_addr(dec, at)?;
                self.mem.write_f(addr, self.r(r1)).map_err(|e| trap_addr(e, at))?;
            }
            Sth => {
                let addr = self.storage_addr(dec, at)?;
                self.mem
                    .write_h(addr, self.r_upper(r1))
                    .map_err(|e| trap_addr(e, at))?;
            }
            Td => {
                let addr = self.storage_addr(dec, at)?;
                let v = self.read_h(addr, at)?;
                let r = v.wrapping_sub(1);
                self.mem.write_h(addr, r).map_err(|e| trap_addr(e, at))?;
                self.cc_value16(r);
            }

            // ---- branching (§5) ----
            Bal | Balr => {
                // First the branch address is computed, then PSW bits 0-31
                // are loaded into R1 (§5.1). BALR with R2=0 takes no branch.
                let target = match dec.operand {
                    Operand::R(0) => None,
                    Operand::R(r2) => Some(self.r_upper(r2)),
                    _ => Some(self.ea16(dec, at, true)?),
                };
                self.set_r(r1, self.psw.word0());
                if let Some(t) = target {
                    self.psw.ic = t;
                }
            }
            Bix => {
                // R1 bits 0-15 index, bits 16-31 count; index += 1,
                // count -= 1, branch if count prior to update > 0 (§5.2).
                let target = self.ea16(dec, at, true)?;
                let v = self.r(r1);
                let old_count = v as u16 as i16;
                let new = (((v >> 16) as u16).wrapping_add(1) as u32) << 16
                    | (v as u16).wrapping_sub(1) as u32;
                self.set_r(r1, new);
                if old_count > 0 {
                    self.psw.ic = target;
                }
            }
            Bc | Bcr => {
                let taken = self.cc_mask_test(r1);
                let target = match dec.operand {
                    Operand::R(r2) => self.r_upper(r2),
                    _ => self.ea16(dec, at, true)?,
                };
                if taken {
                    self.psw.ic = target;
                }
            }
            Bcre => {
                // Like BCR, but PSW bits 0-15 and 24-31 (IC, BSR, DSR) are
                // replaced from R2 (§5.5) — subroutine return across
                // sector boundaries.
                let r2 = operand_reg(dec);
                if self.cc_mask_test(r1) {
                    let v = self.r(r2);
                    self.psw.ic = (v >> 16) as u16;
                    self.psw.bsr = ((v >> 4) & 0xF) as u8;
                    self.psw.dsr = (v & 0xF) as u8;
                }
            }
            Bcf | Bcb => {
                // Branch by adding (BCF, §5.6) / subtracting (BCB, §5.4)
                // the displacement to/from the updated IC.
                let d = srs_disp(dec);
                if self.cc_mask_test(r1) {
                    self.psw.ic = if dec.instr == Bcf {
                        self.psw.ic.wrapping_add(d)
                    } else {
                        self.psw.ic.wrapping_sub(d)
                    };
                }
            }
            Bct | Bctr | Bctb => {
                // Bits 0-15 of R1 reduced by one; branch when the result is
                // not zero (§5.7/5.8). The low-order 16 bits do not
                // participate.
                let target = match dec.operand {
                    Operand::R(r2) => self.r_upper(r2),
                    Operand::Srs { .. } => self.psw.ic.wrapping_sub(srs_disp(dec)),
                    _ => self.ea16(dec, at, true)?,
                };
                let count = self.r_upper(r1).wrapping_sub(1);
                self.set_r_upper(r1, count);
                if count != 0 {
                    self.psw.ic = target;
                }
            }
            Bvc | Bvcr | Bvcf => {
                // M1 bit 6 tests carry (PSW 18), bit 7 tests overflow
                // (PSW 19); M1 bit 5 inverts to test for zero. The overflow
                // indicator is set to 0 by this instruction (§5.9/5.10).
                let m = r1;
                let hits =
                    (m & 0b010 != 0 && self.psw.carry) || (m & 0b001 != 0 && self.psw.overflow);
                let taken = if m & 0b100 != 0 { !hits } else { hits };
                let target = match dec.operand {
                    Operand::R(r2) => Some(self.r_upper(r2)),
                    Operand::Srs { .. } => Some(self.psw.ic.wrapping_add(srs_disp(dec))),
                    _ => Some(self.ea16(dec, at, true)?),
                };
                self.psw.overflow = false;
                if taken {
                    if let Some(t) = target {
                        self.psw.ic = t;
                    }
                }
            }

            // ---- shifts (§6) ----
            Nct => {
                // Normalize R2 left until bit 0 != bit 1, counting in R1
                // bits 0-15; carry 0 if R2 was zero, else 1 (§6.1).
                let r2 = operand_reg(dec);
                let v = self.r(r2);
                if v == 0 {
                    self.set_r(r1, 0);
                    self.psw.carry = false;
                } else {
                    let mut v = v;
                    let mut count: u32 = 0;
                    while (v >> 31) == ((v >> 30) & 1) {
                        v <<= 1;
                        count += 1;
                    }
                    self.set_r(r2, v);
                    self.set_r(r1, count << 16);
                    self.psw.carry = true;
                }
            }
            Sll | Sra | Srl | Srr | Sldl | Srda | Srdl | Srdr => {
                self.shift(dec)?;
            }

            // ---- logical (§7) ----
            N | Nr => {
                let op = self.fetch_full(dec, at)?;
                let r = self.r(r1) & op;
                self.set_r(r1, r);
                self.cc_logical32(r);
            }
            O | Or => {
                let op = self.fetch_full(dec, at)?;
                let r = self.r(r1) | op;
                self.set_r(r1, r);
                self.cc_logical32(r);
            }
            X | Xr => {
                let op = self.fetch_full(dec, at)?;
                let r = self.r(r1) ^ op;
                self.set_r(r1, r);
                self.cc_logical32(r);
            }
            Nhi | Ohi | Xhi => {
                let op = (dec.imm as u32) << 16;
                let a = self.r(r1);
                let r = match dec.instr {
                    Nhi => a & op,
                    Ohi => a | op,
                    _ => a ^ op,
                };
                self.set_r(r1, r);
                self.cc_logical32(r);
            }
            Nst | Ost | Xst => {
                let addr = self.storage_addr(dec, at)?;
                let op = self.mem.read_f(addr).map_err(|e| trap_addr(e, at))?;
                let a = self.r(r1);
                let r = match dec.instr {
                    Nst => a & op,
                    Ost => a | op,
                    _ => a ^ op,
                };
                self.mem.write_f(addr, r).map_err(|e| trap_addr(e, at))?;
                self.cc_logical32(r);
            }
            Nist | Xist | Sb | Zb => {
                // SI storage-modify ops (§7.3/7.7/7.13/7.18).
                let addr = self.storage_addr(dec, at)?;
                let v = self.read_h(addr, at)?;
                let r = match dec.instr {
                    Nist => v & dec.imm,
                    Xist => v ^ dec.imm,
                    Sb => v | dec.imm,
                    _ => v & !dec.imm,
                };
                self.mem.write_h(addr, r).map_err(|e| trap_addr(e, at))?;
                self.cc_logical16(r);
            }
            Zrb => {
                // Zero the register bits selected by the (developed)
                // immediate (§7.19).
                let r = self.r(r1) & !((dec.imm as u32) << 16);
                self.set_r(r1, r);
                self.cc_logical32(r);
            }
            Zh => {
                // Storage halfword set to all zeros; CC not changed (§7.20).
                let addr = self.storage_addr(dec, at)?;
                self.mem.write_h(addr, 0).map_err(|e| trap_addr(e, at))?;
            }
            Shw => {
                // Storage halfword set to all ones; CC not changed (§7.14).
                let addr = self.storage_addr(dec, at)?;
                self.mem.write_h(addr, 0xFFFF).map_err(|e| trap_addr(e, at))?;
            }
            Tb => {
                let addr = self.storage_addr(dec, at)?;
                let v = self.read_h(addr, at)?;
                self.cc_test(v as u32, dec.imm as u32);
            }
            Th => {
                // TEST BITS with an implied all-ones mask (§7.17).
                let addr = self.storage_addr(dec, at)?;
                let v = self.read_h(addr, at)?;
                self.cc_test(v as u32, 0xFFFF);
            }
            Trb => {
                let mask = (dec.imm as u32) << 16;
                self.cc_test(self.r(r1), mask);
            }
            Sum => {
                self.search_under_mask(dec, at)?;
            }
        }
        Ok(())
    }

    /// BC-family condition test (§5.3): M1 bit 5 tests CC=00, bit 6 tests
    /// CC=11, bit 7 tests CC=01; any successful test branches.
    fn cc_mask_test(&self, m1: u8) -> bool {
        (m1 & 0b100 != 0 && self.psw.cc == cc::ZERO)
            || (m1 & 0b010 != 0 && self.psw.cc == cc::NEG)
            || (m1 & 0b001 != 0 && self.psw.cc == cc::POS)
    }

    /// Fractional fullword multiply (§4.21): 64-bit product of two 32-bit
    /// fractions is (a*b) << 1; even R1 receives the pair, odd R1 only the
    /// most significant half. Overflow only for (-1) x (-1). Returns
    /// whether overflow occurred; CC is not changed.
    fn multiply_frac32(&mut self, r1: u8, op: u32) -> bool {
        let a = self.r(r1) as i32 as i64;
        let b = op as i32 as i64;
        let p = ((a * b) as u64) << 1;
        let ovf = a == i32::MIN as i64 && b == i32::MIN as i64;
        if ovf {
            self.psw.overflow = true;
        }
        self.set_r(r1, (p >> 32) as u32);
        if r1 % 2 == 0 {
            self.set_r((r1 + 1) % 8, p as u32);
        }
        ovf
    }

    /// Fractional halfword multiply (§4.22/4.23): 32-bit product fraction
    /// of two halfword fractions, into all of R1.
    fn multiply_frac16(&mut self, r1: u8, b: u16) -> bool {
        let a = (self.r(r1) >> 16) as u16 as i16 as i32;
        let b = b as i16 as i32;
        let p = ((a * b) as u32) << 1;
        let ovf = a == i16::MIN as i32 && b == i16::MIN as i32;
        if ovf {
            self.psw.overflow = true;
        }
        self.set_r(r1, p);
        ovf
    }

    /// Fractional divide (§4.10): 64-bit dividend in R1:(R1+1)mod8 (odd R1:
    /// R1 with 32 low-order zeros appended); unrounded quotient to R1.
    /// Overflow (quotient unrepresentable or divide by zero) leaves the
    /// registers unchanged — the manual calls them indeterminate; this
    /// emulator's deterministic choice is documented in ISA_STATUS.md.
    fn divide_frac(&mut self, r1: u8, divisor: i32) -> bool {
        let dividend: i64 = if r1 % 2 == 1 {
            (self.r(r1) as i64) << 32
        } else {
            (((self.r(r1) as u64) << 32) | self.r((r1 + 1) % 8) as u64) as i64
        };
        if divisor == 0 {
            self.psw.overflow = true;
            return true;
        }
        let q = dividend as i128 / (divisor as i128 * 2);
        if q < i32::MIN as i128 || q > i32::MAX as i128 {
            self.psw.overflow = true;
            return true;
        }
        self.set_r(r1, q as i32 as u32);
        false
    }

    /// Shift execution (§6). The count field selects an immediate count
    /// (1-55), a computed count from bits 10-15 of GR0-GR7 (field values
    /// 56-63), or no operation (0) — Figure 6-1.
    fn shift(&mut self, dec: &Decoded) -> Result<(), Trap> {
        use Instr::*;
        let count_field = match dec.operand {
            Operand::Count(c) => c,
            _ => unreachable!(),
        };
        let n: u32 = match count_field {
            0 => return Ok(()),
            1..=55 => count_field as u32,
            _ => (self.r(count_field - 56) >> 16) as u32 & 0x3F,
        };
        if n == 0 {
            return Ok(());
        }
        let r1 = dec.r1;
        let pair = (r1 + 1) % 8;
        match dec.instr {
            Sll => {
                let v = self.r(r1);
                // Bits leaving the high-order position enter the carry
                // indicator; the carry ends as the last bit shifted out
                // (§6.2).
                self.psw.carry = if n <= 32 { (v >> (32 - n)) & 1 != 0 } else { false };
                self.set_r(r1, if n < 32 { v << n } else { 0 });
            }
            Srl => {
                let v = self.r(r1);
                self.set_r(r1, if n < 32 { v >> n } else { 0 });
            }
            Sra => {
                let v = self.r(r1) as i32;
                self.set_r(r1, (v >> n.min(31)) as u32);
            }
            Srr => {
                let v = self.r(r1);
                self.set_r(r1, v.rotate_right(n % 32));
            }
            Sldl => {
                let v = ((self.r(r1) as u64) << 32) | self.r(pair) as u64;
                self.psw.carry = if n <= 64 { (v >> (64 - n)) & 1 != 0 } else { false };
                let r = if n < 64 { v << n } else { 0 };
                self.set_r(r1, (r >> 32) as u32);
                self.set_r(pair, r as u32);
            }
            Srdl => {
                let v = ((self.r(r1) as u64) << 32) | self.r(pair) as u64;
                let r = if n < 64 { v >> n } else { 0 };
                self.set_r(r1, (r >> 32) as u32);
                self.set_r(pair, r as u32);
            }
            Srda => {
                let v = (((self.r(r1) as u64) << 32) | self.r(pair) as u64) as i64;
                let r = (v >> n.min(63)) as u64;
                self.set_r(r1, (r >> 32) as u32);
                self.set_r(pair, r as u32);
            }
            Srdr => {
                let v = ((self.r(r1) as u64) << 32) | self.r(pair) as u64;
                let r = v.rotate_right(n % 64);
                self.set_r(r1, (r >> 32) as u32);
                self.set_r(pair, r as u32);
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    /// SEARCH UNDER MASK (§7.12): compare (Ai AND M) with (FV AND M) for
    /// `count` array elements addressed via R1's address/modifier halves;
    /// mask and field values in (R1+1)mod8.
    fn search_under_mask(&mut self, dec: &Decoded, at: u32) -> Result<(), Trap> {
        let r1 = dec.r1;
        let r2 = operand_reg(dec);
        let count = (self.r(r2) >> 16) as u16 as i16;
        let pairv = self.r((r1 + 1) % 8);
        let mask = (pairv >> 16) as u16;
        let fv = pairv as u16;
        let want = fv & mask;
        let mut ptr = (self.r(r1) >> 16) as u16;
        let inc = self.r(r1) as u16;
        for _ in 0..count.max(0) {
            let y = self.read_h(self.expand_data(ptr, None), at)?;
            if (y & mask) ^ want != 0 {
                self.set_r_upper(r1, ptr);
                self.psw.cc = cc::NEG;
                return Ok(());
            }
            ptr = ptr.wrapping_add(inc);
        }
        self.set_r_upper(r1, ptr);
        self.psw.cc = cc::ZERO;
        Ok(())
    }
}

fn operand_reg(dec: &Decoded) -> u8 {
    match dec.operand {
        Operand::R(r2) => r2,
        _ => unreachable!("RR-only instruction"),
    }
}

fn srs_disp(dec: &Decoded) -> u16 {
    match dec.operand {
        Operand::Srs { d, .. } => d as u16,
        _ => unreachable!("SRS-only branch form"),
    }
}

fn trap_addr(e: AddressError, at: u32) -> Trap {
    Trap::Address { addr: e.addr, at }
}
