//! AP-101S CPU: machine state, effective-address generation, and execution
//! of the phase-1 instruction subset.
//!
//! All semantics cite IBM 85-C67-001 (see docs/SOURCES.md); section numbers
//! in comments refer to that manual.

use crate::decode::{self, Decoded, Instr, Operand, Width};
use crate::float::{self, FpEvent, Precision, Unpacked};
use crate::mem::{AddressError, Memory};
use crate::psw::{cc, Psw};

/// Preferred storage area old/new PSW locations (§2.5.2, Figure 2-20/2-21).
pub mod psa {
    /// Program-exception class (illegal op, privileged op, fixed/floating
    /// overflow, underflow, significance, divide, convert overflow).
    pub const PROGRAM_OLD: u32 = 0x0048;
    pub const PROGRAM_NEW: u32 = 0x004C;
    /// Supervisor call.
    pub const SVC_OLD: u32 = 0x0058;
    pub const SVC_NEW: u32 = 0x005C;
    /// Instruction monitor (CPU breakpoint), Figure 2-20.
    pub const MONITOR_OLD: u32 = 0x0070;
    pub const MONITOR_NEW: u32 = 0x0074;
    /// System-class interrupt levels (§2.5.2, Figure 2-20/2-21):
    /// (old PSW, new PSW, PSW system-mask bit as a mask over bits 32-39).
    /// In mask-bit order — the lowest-numbered bit is taken first
    /// (§2.5.2 item 5).
    pub const SYSTEM_LEVELS: [(u32, u32, u8); 7] = [
        (0x0060, 0x0064, 0x80), // counter/interval timer 1 (bit 32)
        (0x0068, 0x006C, 0x40), // interval timer 2 (bit 33)
        (0x0078, 0x007C, 0x10), // external 0 (bit 35)
        (0x0080, 0x0084, 0x08), // external 1 (bit 36)
        (0x0088, 0x008C, 0x04), // external 2 — IOP programmed (bit 37)
        (0x0090, 0x0094, 0x02), // external 3 (bit 38)
        (0x0098, 0x009C, 0x01), // external 4 (bit 39)
    ];
}

/// Program-exception interrupt codes (§2.5.2 Figure 2-20).
pub mod pe_code {
    pub const ILLEGAL: u16 = 0x0000;
    pub const PRIVILEGED: u16 = 0x0001;
    pub const FIXED_OVERFLOW: u16 = 0x0004;
    pub const SIGNIFICANCE: u16 = 0x0005;
    pub const FP_UNDERFLOW: u16 = 0x0009;
    pub const CONVERT_OVERFLOW: u16 = 0x000A;
    pub const FP_OVERFLOW: u16 = 0x000B;
    pub const FP_DIVIDE: u16 = 0x000C;
    pub const STORE_PROTECT: u16 = 0x0007;
}

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
    /// An interrupt fired but its PSA "new PSW" doubleword is all zero —
    /// no handler was installed. Architecturally the machine would load a
    /// zero PSW and execute from address 0; the emulator halts with the
    /// pending interrupt code instead (documented convention).
    UninitializedInterrupt { code: u16, at: u32 },
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
    /// Attached I/O subsystem for the PC instruction (§3.3); with none
    /// attached, PC operations time out (CC 01) — architecturally the
    /// handshake never completes.
    pub io: Option<Box<dyn IoSubsystem>>,
    /// Pending system-class interrupts, one per `psa::SYSTEM_LEVELS`
    /// entry. System interrupts remain pending while masked (§2.5.2.3)
    /// and are taken at ENDOP when their mask bit allows.
    pub pending_system: [bool; 7],
}

impl Default for Cpu {
    fn default() -> Cpu {
        Cpu::new(Memory::full())
    }
}

/// A program-controlled I/O subsystem attached to the CPU (§3.2-3.3): the
/// seam where the IOP model (phase 3) and inter-GPC channels (phase 4)
/// plug in. The PC instruction transmits a 32-bit control word and either
/// sends or receives one fullword.
pub trait IoSubsystem {
    /// Handle one PC operation. `data` is `Some` for output operations
    /// (CW bit 0 = 1). Return the input word for input operations,
    /// `OutputAccepted` for outputs, or `Timeout` if the handshake fails
    /// (§3.3: sets CC 01).
    fn pc(&mut self, cw: u32, data: Option<u32>) -> PcResponse;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PcResponse {
    Input(u32),
    OutputAccepted,
    Timeout,
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
            io: None,
            pending_system: [false; 7],
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
        if instr.halfword_index_alignment() {
            return idx;
        }
        match instr.width() {
            Width::Half => idx,
            Width::Full => idx << 1,
            Width::Double => idx << 2,
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
                    // Long floating point has no SRS forms (§8 encodings);
                    // unreachable but harmless.
                    Width::Double => (d as u16) << 2,
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
                    // Fullword indirect with automatic storage
                    // modification (§2.2.8 step 6, Figure 2-15): the
                    // pointer fullword holds address (bits 0-15) and
                    // modifier (bits 16-31); after the EA is formed the
                    // modifier is added to the address and written back.
                    (0, true, true) => {
                        let ptr_addr = self.expand_data(pea, base_reg);
                        let ptr =
                            self.mem.read_f(ptr_addr).map_err(|e| trap_addr(e, at))?;
                        let ea16 = (ptr >> 16) as u16;
                        let addr = if branch {
                            self.expand_branch(ea16)
                        } else {
                            self.expand_data(ea16, None)
                        };
                        let modified = ea16.wrapping_add(ptr as u16) as u32;
                        self.mem
                            .write_f(ptr_addr, (modified << 16) | (ptr & 0xFFFF))
                            .map_err(|e| trap_addr(e, at))?;
                        return Ok(Ea::Mem { ea16, addr });
                    }
                    // Fullword indirect address pointer with postindexing
                    // (X!=0, IA=1, I=1): §2.2.8 step 10, Figure 2-17.
                    // Pointer fullword: address in bits 1-15, Xc (index
                    // suppress) bit 20, C bit 21 (with CB/CD bits 22/23
                    // selecting PSW BSR/DSR replacement from the BSV/DSV
                    // fields, bits 24-31). Semantics cross-checked
                    // against yaGPC2's cpu_g_ea.
                    (x, true, true) => {
                        let ptr_addr = self.expand_data(pea, base_reg);
                        let fw =
                            self.mem.read_f(ptr_addr).map_err(|e| trap_addr(e, at))?;
                        let addr15 = (fw >> 16) & 0x7FFF;
                        let xc = (fw >> 11) & 1;
                        let c = (fw >> 10) & 1;
                        let cb = (fw >> 9) & 1;
                        let cd = (fw >> 8) & 1;
                        let bsv = (fw >> 4) as u8 & 0xF;
                        let dsv = fw as u8 & 0xF;
                        if c == 1 {
                            if cd == 1 {
                                self.psw.dsr = dsv;
                            }
                            if cb == 1 {
                                self.psw.bsr = bsv;
                            }
                        }
                        let eff_dsr = if c == 0 { dsv } else { self.psw.dsr };
                        let off = if xc == 0 {
                            (addr15 + self.aligned_index(x, instr) as u32) & 0x7FFF
                        } else {
                            addr15
                        };
                        let sector =
                            (if branch { self.psw.bsr } else { eff_dsr }) as u32;
                        // Branch consumers store ea16 into the IC and
                        // re-expand through the BSR: keep bit 15 set so
                        // the sector survives the round trip.
                        let ea16 =
                            if branch { 0x8000 | off as u16 } else { off as u16 };
                        return Ok(Ea::Mem { ea16, addr: (sector << 15) | off });
                    }
                    // remaining combinations are covered by the guarded
                    // arms above
                    _ => unreachable!("RS-indexed mode combinations exhausted"),
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

    // ----- interrupts (§2.5.2) -----

    /// PSW swap: store the current PSW at `old`, load the PSW at `new`
    /// (§2.5.2.1). If the new-PSW doubleword is uninitialized (all zero),
    /// the emulator halts with `UninitializedInterrupt` instead of
    /// executing from a zero PSW.
    fn psw_swap(&mut self, old: u32, new: u32, at: u32) -> Result<(), Trap> {
        let w0 = self.mem.read_f(new).map_err(|e| trap_addr(e, at))?;
        let w1 = self.mem.read_f(new + 2).map_err(|e| trap_addr(e, at))?;
        if w0 == 0 && w1 == 0 {
            return Err(Trap::UninitializedInterrupt { code: self.psw.int_code, at });
        }
        self.mem.write_f(old, self.psw.word0()).map_err(|e| trap_addr(e, at))?;
        let word1 = self.psw.word1();
        self.mem.write_f(old + 2, word1).map_err(|e| trap_addr(e, at))?;
        self.psw.set_word0(w0);
        self.psw.set_word1(w1);
        Ok(())
    }

    /// Program-exception interrupt with the given code (Figure 2-20). The
    /// code is recorded in the stored (old) PSW.
    fn program_interrupt(&mut self, code: u16, at: u32) -> Result<(), Trap> {
        self.psw.int_code = code;
        self.psw_swap(psa::PROGRAM_OLD, psa::PROGRAM_NEW, at)
    }

    /// Store honoring storage protection (§2.4): a protected halfword
    /// causes an (unmaskable) program interrupt, code 0007, and the store
    /// does not occur. Returns false when the interrupt was taken — the
    /// instruction terminates without its remaining effects.
    fn store_h_prot(&mut self, addr: u32, v: u16, at: u32) -> Result<bool, Trap> {
        if self.mem.is_protected(addr) {
            self.program_interrupt(pe_code::STORE_PROTECT, at)?;
            return Ok(false);
        }
        self.mem.write_h(addr, v).map_err(|e| trap_addr(e, at))?;
        Ok(true)
    }

    fn store_f_prot(&mut self, addr: u32, v: u32, at: u32) -> Result<bool, Trap> {
        if self.mem.is_protected(addr) || self.mem.is_protected(addr.wrapping_add(1)) {
            self.program_interrupt(pe_code::STORE_PROTECT, at)?;
            return Ok(false);
        }
        self.mem.write_f(addr, v).map_err(|e| trap_addr(e, at))?;
        Ok(true)
    }

    // ----- fetch/execute -----

    /// Execute one instruction. Returns the 19-bit address it was fetched
    /// from.
    pub fn step(&mut self) -> Result<u32, Trap> {
        let at = self.expand_branch(self.psw.ic);
        // Instruction monitor (§2.4.1): with PSW bit 34 set, executing an
        // unprotected instruction word interrupts; the AP-101S leaves the
        // IC pointing at the offending instruction.
        if self.psw.sys_mask & 0b0010_0000 != 0 && !self.mem.is_protected(at) {
            self.psw_swap(psa::MONITOR_OLD, psa::MONITOR_NEW, at)?;
            return Ok(at);
        }
        let hw1 = self.read_h(at, at)?;
        // The second halfword is fetched only if the format needs it.
        let hw2 = self.mem.read_h(at.wrapping_add(1)).unwrap_or(0);
        let dec = match decode::decode(hw1, hw2) {
            Ok(d) => d,
            Err(decode::DecodeError::Illegal { hw1 }) => {
                // Illegal instruction: unmaskable program exception, code
                // 0000 (Figure 2-20). IC is left at the offending
                // instruction (the table marks the stored PC "can vary").
                return self
                    .program_interrupt(pe_code::ILLEGAL, at)
                    .map(|()| at)
                    .map_err(|t| match t {
                        Trap::UninitializedInterrupt { .. } => {
                            Trap::IllegalInstruction { hw1, at }
                        }
                        other => other,
                    });
            }
        };
        if dec.len == 2 {
            // Ensure the second halfword really was addressable.
            self.read_h(at.wrapping_add(1), at)?;
        }
        // The PSW reflects the updated IC during execution: BAL links to
        // the next sequential instruction (§5.1) and IC-relative/BCF/BCB
        // arithmetic uses the updated IC (§2.2.8, §5.4, §5.6).
        self.psw.ic = self.psw.ic.wrapping_add(dec.len as u16);
        self.exec(&dec, at)?;
        // ENDOP fixed-point overflow interrupt: taken whenever the overflow
        // indicator (PSW 19) and its mask (PSW 20) are both set — including
        // via SPM/LPS loading such a PSW (§2.5.2.3 note, §9.3/9.5).
        if self.psw.overflow && self.psw.fixed_overflow_mask {
            self.program_interrupt(pe_code::FIXED_OVERFLOW, at)?;
        }
        self.deliver_system_interrupts(at)?;
        self.steps += 1;
        Ok(at)
    }

    /// Take pending unmasked system interrupts at ENDOP, lowest mask-bit
    /// number first (§2.5.2). Masked levels stay pending. The interrupt
    /// code for external levels is delivered as 0000 (Figure 2-20).
    pub fn deliver_system_interrupts(&mut self, at: u32) -> Result<(), Trap> {
        for (i, (old, new, mask)) in psa::SYSTEM_LEVELS.iter().enumerate() {
            if self.pending_system[i] && self.psw.sys_mask & mask != 0 {
                self.pending_system[i] = false;
                self.psw.int_code = 0x0000;
                self.psw_swap(*old, *new, at)?;
            }
        }
        Ok(())
    }

    /// Run until a halt condition. A taken branch that targets itself
    /// (tight loop) halts — the conventional way for test programs to stop.
    pub fn run(&mut self, max_steps: u64) -> Halt {
        for _ in 0..max_steps {
            if self.psw.wait {
                // §2.5.4: the wait state is interruptible when not masked.
                let at = self.expand_branch(self.psw.ic);
                match self.deliver_system_interrupts(at) {
                    Ok(()) => {}
                    Err(t) => return Halt::Trap(t),
                }
                if self.psw.wait {
                    return Halt::Wait;
                }
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
                let (r, _) = self.add_flags(self.r(r1), op);
                self.set_r(r1, r);
                self.cc_value32(r);
            }
            Ah => {
                let op = self.fetch_half_developed(dec, at)?;
                let (r, _) = self.add_flags(self.r(r1), op);
                self.set_r(r1, r);
                self.cc_value32(r);
            }
            Ahi => {
                let op = (dec.imm as u32) << 16;
                let (r, _) = self.add_flags(self.r(r1), op);
                self.set_r(r1, r);
                self.cc_value32(r);
            }
            Ast => {
                // R1 + second operand -> second operand location (§4.4).
                let addr = self.storage_addr(dec, at)?;
                let op = self.mem.read_f(addr).map_err(|e| trap_addr(e, at))?;
                let (r, _) = self.add_flags(self.r(r1), op);
                if !self.store_f_prot(addr, r, at)? {
                    return Ok(());
                }
                self.cc_value32(r);
            }
            S | Sr => {
                let op = self.fetch_full(dec, at)?;
                let (r, _) = self.sub_flags(self.r(r1), op);
                self.set_r(r1, r);
                self.cc_value32(r);
            }
            Sh => {
                let op = self.fetch_half_developed(dec, at)?;
                let (r, _) = self.sub_flags(self.r(r1), op);
                self.set_r(r1, r);
                self.cc_value32(r);
            }
            Sst => {
                // R1 subtracted FROM the second operand; result to storage
                // (§4.29).
                let addr = self.storage_addr(dec, at)?;
                let op = self.mem.read_f(addr).map_err(|e| trap_addr(e, at))?;
                let (r, _) = self.sub_flags(op, self.r(r1));
                if !self.store_f_prot(addr, r, at)? {
                    return Ok(());
                }
                self.cc_value32(r);
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
                self.multiply_frac32(r1, op);
            }
            Mh => {
                let addr = self.storage_addr(dec, at)?;
                let b = self.read_h(addr, at)?;
                self.multiply_frac16(r1, b);
            }
            Mhi => {
                self.multiply_frac16(r1, dec.imm);
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
            }
            D | Dr => {
                let op = self.fetch_full(dec, at)?;
                self.divide_frac(r1, op as i32);
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
                    if !self.store_f_prot(addr.wrapping_add(2 * n as u32), self.r(n), at)? {
                        return Ok(());
                    }
                }
            }
            Msth => {
                // Immediate (twos complement) added to the halfword storage
                // operand (§4.20).
                let addr = self.storage_addr(dec, at)?;
                let v = self.read_h(addr, at)?;
                let r = v.wrapping_add(dec.imm);
                if !self.store_h_prot(addr, r, at)? {
                    return Ok(());
                }
                self.cc_value16(r);
            }
            St => {
                let addr = self.storage_addr(dec, at)?;
                self.store_f_prot(addr, self.r(r1), at)?;
            }
            Sth => {
                let addr = self.storage_addr(dec, at)?;
                let v = self.r_upper(r1);
                self.store_h_prot(addr, v, at)?;
            }
            Td => {
                let addr = self.storage_addr(dec, at)?;
                let v = self.read_h(addr, at)?;
                let r = v.wrapping_sub(1);
                if !self.store_h_prot(addr, r, at)? {
                    return Ok(());
                }
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
                if !self.store_f_prot(addr, r, at)? {
                    return Ok(());
                }
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
                if !self.store_h_prot(addr, r, at)? {
                    return Ok(());
                }
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
                self.store_h_prot(addr, 0, at)?;
            }
            Shw => {
                // Storage halfword set to all ones; CC not changed (§7.14).
                let addr = self.storage_addr(dec, at)?;
                self.store_h_prot(addr, 0xFFFF, at)?;
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

            // ---- floating point (§8) ----
            Aer | Ae | Ser | Se => {
                let a = self.fpr_short(r1);
                let b = self.fp_operand_short(dec, at)?;
                let negate = matches!(dec.instr, Ser | Se);
                let (r, ev) = float::add(a, b, negate, Precision::Short);
                self.fp_finish_short(r1, r, ev, true, at)?;
            }
            Aedr | Aed | Sedr | Sed => {
                let a = self.fpr_long(r1);
                let b = self.fp_operand_long(dec, at)?;
                let negate = matches!(dec.instr, Sedr | Sed);
                let (r, ev) = float::add(a, b, negate, Precision::Long);
                self.fp_finish_long(r1, r, ev, true, at)?;
            }
            Cer | Ce => {
                let a = self.fpr_short(r1);
                let b = self.fp_operand_short(dec, at)?;
                self.fp_cc_compare(float::compare(a, b, Precision::Short));
            }
            Cedr | Ced => {
                let a = self.fpr_long(r1);
                let b = self.fp_operand_long(dec, at)?;
                self.fp_cc_compare(float::compare(a, b, Precision::Long));
            }
            Mer | Me => {
                // §8.25: even R1 receives the full-precision product in the
                // register pair; odd R1 a 32-bit product. CC unchanged.
                let a = self.fpr_short(r1);
                let b = self.fp_operand_short(dec, at)?;
                let p = if r1 % 2 == 0 { Precision::Long } else { Precision::Short };
                let (r, ev) = float::multiply(a, b, p);
                if r1 % 2 == 0 {
                    self.fp_finish_long(r1, r, ev, false, at)?;
                } else {
                    self.fp_finish_short(r1, r, ev, false, at)?;
                }
            }
            Medr | Med => {
                let a = self.fpr_long(r1);
                let b = self.fp_operand_long(dec, at)?;
                let (r, ev) = float::multiply(a, b, Precision::Long);
                self.fp_finish_long(r1, r, ev, false, at)?;
            }
            Der | De => {
                let a = self.fpr_short(r1);
                let b = self.fp_operand_short(dec, at)?;
                let (r, ev) = float::divide(a, b, Precision::Short);
                self.fp_finish_short(r1, r, ev, false, at)?;
            }
            Dedr | Ded => {
                let a = self.fpr_long(r1);
                let b = self.fp_operand_long(dec, at)?;
                let (r, ev) = float::divide(a, b, Precision::Long);
                self.fp_finish_long(r1, r, ev, false, at)?;
            }
            Ler | Le => {
                // §8.18: loads do not normalize; CC from the fraction only.
                let w = match dec.operand {
                    Operand::R(r2) => self.fpr[(r2 & 7) as usize],
                    _ => {
                        let addr = self.storage_addr(dec, at)?;
                        self.mem.read_f(addr).map_err(|e| trap_addr(e, at))?
                    }
                };
                self.fpr[(r1 & 7) as usize] = w;
                self.fp_cc_value(float::unpack_short(w));
            }
            Led => {
                let addr = self.storage_addr(dec, at)?;
                let hi = self.mem.read_f(addr).map_err(|e| trap_addr(e, at))?;
                let lo = self.mem.read_f(addr + 2).map_err(|e| trap_addr(e, at))?;
                self.fpr[(r1 & 7) as usize] = hi;
                self.fpr[((r1 + 1) % 8) as usize] = lo;
                self.fp_cc_value(float::unpack_long(hi, lo));
            }
            Lecr => {
                // §8.19: sign inverted; a zero-fraction operand loads as a
                // true zero; (R1+1) unchanged.
                let r2 = operand_reg(dec);
                let w = self.fpr[(r2 & 7) as usize];
                let out = if w & 0x00FF_FFFF == 0 { 0 } else { w ^ 0x8000_0000 };
                self.fpr[(r1 & 7) as usize] = out;
                self.fp_cc_value(float::unpack_short(out));
            }
            Ste => {
                let addr = self.storage_addr(dec, at)?;
                self.store_f_prot(addr, self.fpr[(r1 & 7) as usize], at)?;
            }
            Sted => {
                let addr = self.storage_addr(dec, at)?;
                if !self.store_f_prot(addr, self.fpr[(r1 & 7) as usize], at)? {
                    return Ok(());
                }
                self.store_f_prot(addr + 2, self.fpr[((r1 + 1) % 8) as usize], at)?;
            }
            Cvfx => {
                // §8.13: unnormalize to characteristic 0x44 and convert to a
                // twos-complement fixed value with the binary point between
                // bits 15 and 16, truncated. Out-of-range: convert overflow.
                let r2 = operand_reg(dec);
                let u = self.fpr_short(r2);
                if u.is_zero() {
                    self.set_r(r1, 0);
                    self.psw.cc = cc::ZERO;
                } else {
                    // value * 2^16 = frac(56-bit) * 16^(ch-64) / 16^14 * 2^16
                    let shift = 4 * u.ch - 296; // 4*(ch-64) + 16 - 56
                    let mag: i128 = if shift >= 0 {
                        (u.frac as i128) << shift.min(80)
                    } else {
                        (u.frac as i128) >> (-shift).min(127)
                    };
                    let v = if u.neg { -mag } else { mag };
                    if v > i32::MAX as i128 || v < i32::MIN as i128 {
                        return self.program_interrupt(pe_code::CONVERT_OVERFLOW, at);
                    }
                    let v = v as i32 as u32;
                    self.set_r(r1, v);
                    self.cc_value16((v >> 16) as u16);
                }
            }
            Cvfl => {
                // §8.14: fixed (binary point between bits 15/16) to short
                // float via characteristic 0x44 and normalization.
                let r2 = operand_reg(dec);
                let v = self.r(r2) as i32;
                if v == 0 {
                    self.fpr[(r1 & 7) as usize] = 0;
                    self.psw.cc = cc::ZERO;
                } else {
                    let u = Unpacked {
                        neg: v < 0,
                        ch: 0x44,
                        frac: (v.unsigned_abs() as u64) << 24,
                    };
                    let (n, _) = float::normalize_value(u, Precision::Short);
                    self.fpr[(r1 & 7) as usize] = float::pack_short(n);
                    self.fp_cc_value(n);
                }
            }
            Mvs => {
                // §8.23: midvalue of FPR R1, FPR (R1+1) [upper limit] and
                // the storage operand [lower limit]; CC per the limiter
                // table; the normalized midvalue replaces R1.
                let v = self.fpr_short(r1);
                let upper = self.fpr_short((r1 + 1) % 8);
                let addr = self.storage_addr(dec, at)?;
                let m = self.mem.read_f(addr).map_err(|e| trap_addr(e, at))?;
                let lower = float::unpack_short(m);
                let (sel, code) = if float::compare(v, upper, Precision::Short) > 0 {
                    (upper, cc::POS)
                } else if float::compare(v, lower, Precision::Short) < 0 {
                    (lower, cc::NEG)
                } else {
                    (v, cc::ZERO)
                };
                self.psw.cc = code;
                let (n, ev) = float::normalize_value(sel, Precision::Short);
                match ev {
                    FpEvent::Underflow if self.psw.exp_underflow_mask => {
                        return self.program_interrupt(pe_code::FP_UNDERFLOW, at);
                    }
                    FpEvent::Underflow => self.fpr[(r1 & 7) as usize] = 0,
                    _ => self.fpr[(r1 & 7) as usize] = float::pack_short(n),
                }
            }
            Lfli => {
                // §8.21: value code 0 = true zero, n = float n (0x41n00000).
                let code = dec.imm as u32 & 0xF;
                self.fpr[(r1 & 7) as usize] =
                    if code == 0 { 0 } else { 0x4100_0000 | (code << 20) };
            }
            Lflr => {
                let r2 = operand_reg(dec);
                self.fpr[(r1 & 7) as usize] = self.r(r2);
            }
            Lfxr => {
                let r2 = operand_reg(dec);
                self.set_r(r1, self.fpr[(r2 & 7) as usize]);
            }

            // ---- status switching (§2.5, §9) ----
            Lps => {
                // §9.3: privileged; two fullwords replace the PSW. CC and
                // indicators come from the new PSW; the ENDOP fixed-point
                // overflow check applies to the loaded PSW.
                if self.privileged_violation(at)? {
                    return Ok(());
                }
                let addr = self.storage_addr(dec, at)?;
                let w0 = self.mem.read_f(addr).map_err(|e| trap_addr(e, at))?;
                let w1 = self.mem.read_f(addr + 2).map_err(|e| trap_addr(e, at))?;
                self.psw.set_word0(w0);
                self.psw.set_word1(w1);
            }
            Spm => {
                // §9.5: R2 bits 16-23 replace CC, carry, overflow, and the
                // three arithmetic masks.
                let r2 = operand_reg(dec);
                let v = self.r(r2);
                self.psw.cc = ((v >> 14) & 3) as u8;
                self.psw.carry = v & (1 << 13) != 0;
                self.psw.overflow = v & (1 << 12) != 0;
                self.psw.fixed_overflow_mask = v & (1 << 11) != 0;
                self.psw.exp_underflow_mask = v & (1 << 9) != 0;
                self.psw.significance_mask = v & (1 << 8) != 0;
            }
            Ssm => {
                // §9.6: privileged; the halfword operand replaces PSW bits
                // 32-47 (system mask, EA-high, register set, machine check
                // mask, wait, problem state).
                if self.privileged_violation(at)? {
                    return Ok(());
                }
                let addr = self.storage_addr(dec, at)?;
                let hw = self.read_h(addr, at)?;
                let word1 = ((hw as u32) << 16) | self.psw.int_code as u32;
                self.psw.set_word1(word1);
            }
            Svc => {
                // §9.9: interruption via the SVC PSW pair; the 16-bit EA is
                // the interrupt code and the 4-bit sector extension goes to
                // old-PSW bits 40-43. Cannot be masked.
                let (ea16, addr19) = match self.resolve(dec, at, false)? {
                    Ea::Mem { ea16, addr } => (ea16, addr),
                    Ea::Reg(_) => unreachable!("SVC has storage operand"),
                };
                self.psw.ea_high = ((addr19 >> 15) & 0xF) as u8;
                self.psw.int_code = ea16;
                self.psw_swap(psa::SVC_OLD, psa::SVC_NEW, at)?;
            }
            Ts => {
                // §9.10: test the halfword (three-state CC, all-ones mask)
                // then set it to all ones, atomically.
                let addr = self.storage_addr(dec, at)?;
                let v = self.read_h(addr, at)?;
                self.cc_test(v as u32, 0xFFFF);
                self.store_h_prot(addr, 0xFFFF, at)?;
            }
            Tsb => {
                // §9.11: test bits (three-state CC), then OR the immediate
                // into the halfword operand, atomically.
                let addr = self.storage_addr(dec, at)?;
                let v = self.read_h(addr, at)?;
                self.cc_test(v as u32, dec.imm as u32);
                self.store_h_prot(addr, v | dec.imm, at)?;
            }
            Ispb => {
                // §9.2: privileged; M1 selects set/reset for the halfword
                // or fullword at the EA; M1 with bit 5 set is illegal.
                if self.privileged_violation(at)? {
                    return Ok(());
                }
                if r1 & 0b100 != 0 {
                    return self.program_interrupt(pe_code::ILLEGAL, at);
                }
                let addr = self.storage_addr(dec, at)?;
                let on = r1 & 0b010 != 0;
                if r1 & 0b001 != 0 {
                    // both halfwords of the fullword; EA low bit ignored
                    let base = addr & !1;
                    self.mem.set_protected(base, on).map_err(|e| trap_addr(e, at))?;
                    self.mem.set_protected(base | 1, on).map_err(|e| trap_addr(e, at))?;
                } else {
                    self.mem.set_protected(addr, on).map_err(|e| trap_addr(e, at))?;
                }
            }
            Mvh => {
                // §9.4: block move of `count` halfwords (R1 bits 16-31),
                // from source sector/offset in R2 to destination
                // sector/offset in R1, high address first: MS(D+C) <-
                // MS(S+C). Executed atomically here (no async interrupts
                // yet); a store-protect violation backs the IC up to the
                // MVH itself with the remaining count in R1 (§9.4 notes).
                let r2 = operand_reg(dec);
                let r1v = self.r(r1);
                let r2v = self.r(r2);
                let count = r1v as u16 as i16;
                if count > 0 {
                    let dsect = if r1v & 0x8000_0000 != 0 {
                        self.psw.dsr
                    } else {
                        self.dse[self.psw.reg_set as usize][(r1 & 7) as usize]
                    } as u32;
                    let ssect = if r2v & 0x8000_0000 != 0 {
                        (r2v & 0xF) as u32
                    } else {
                        0
                    };
                    let d19 = (dsect << 15) | ((r2v_dest(r1v)) as u32);
                    let s19 = (ssect << 15) | (((r2v >> 16) & 0x7FFF) as u32);
                    // The count is decremented BEFORE each move (Figure
                    // 9-1 as implemented by the hardware; confirmed via
                    // yaGPC2 exec_MVH): offsets count-1 down to 0.
                    for c in (0..count as u32).rev() {
                        let v = self.read_h(s19 + c, at)?;
                        if self.mem.is_protected(d19 + c) {
                            self.set_r_lower(r1, c as u16 + 1);
                            self.psw.ic = self.psw.ic.wrapping_sub(dec.len as u16);
                            return self.program_interrupt(pe_code::STORE_PROTECT, at);
                        }
                        self.mem.write_h(d19 + c, v).map_err(|e| trap_addr(e, at))?;
                    }
                    self.set_r_lower(r1, 0);
                }
            }
            Scal => {
                // §9.7: compute the branch address, then save PSW bits 0-31
                // and the eight GPRs (18 halfwords) at the stack save area
                // derived from the SSD in R1; update the SSD (PTR += INC,
                // INC = 18); branch.
                let target = self.ea16(dec, at, true)?;
                let ssd = self.r(r1);
                let sector = if ssd & 0x8000_0000 != 0 {
                    self.psw.dsr
                } else {
                    self.dse[self.psw.reg_set as usize][(r1 & 7) as usize]
                } as u32;
                let ptr15 = ((ssd >> 16) & 0x7FFF) as u16;
                let inc = ssd as u16;
                let off = (ptr15.wrapping_add(inc)) & 0x7FFF;
                let sa = (sector << 15) | off as u32;
                if !self.store_f_prot(sa, self.psw.word0(), at)? {
                    return Ok(());
                }
                for n in 0..8u8 {
                    if !self.store_f_prot(sa + 2 + 2 * n as u32, self.r(n), at)? {
                        return Ok(());
                    }
                }
                let new_ssd = (ssd & 0x8000_0000) | ((off as u32) << 16) | 18;
                self.set_r(r1, new_ssd);
                self.psw.ic = target;
            }
            Sret => {
                // §9.8: conditional (M1 vs CC); on branch, PSW bits 0-31
                // and all eight GPRs reload from the stack frame addressed
                // by the SSD in R2.
                let r2 = operand_reg(dec);
                if self.cc_mask_test(r1) {
                    let ssd = self.r(r2);
                    let sector = if ssd & 0x8000_0000 != 0 {
                        self.psw.dsr
                    } else {
                        self.dse[self.psw.reg_set as usize][(r2 & 7) as usize]
                    } as u32;
                    let sa = (sector << 15) | ((ssd >> 16) & 0x7FFF);
                    let w0 = self.mem.read_f(sa).map_err(|e| trap_addr(e, at))?;
                    self.psw.set_word0(w0);
                    for n in 0..8u8 {
                        let v = self
                            .mem
                            .read_f(sa + 2 + 2 * n as u32)
                            .map_err(|e| trap_addr(e, at))?;
                        self.set_r(n, v);
                    }
                }
            }
            Lxar | Lxa => {
                // §9.12: R1 bits 1-15 from the address constant's bits
                // 1-15 (bits 0 and 16-31 zeroed); R1's DSE from its bits
                // 28-31.
                let cv = match dec.operand {
                    Operand::R(r2) => self.r(r2),
                    _ => {
                        let addr = self.storage_addr(dec, at)?;
                        self.mem.read_f(addr).map_err(|e| trap_addr(e, at))?
                    }
                };
                self.set_r(r1, cv & 0x7FFF_0000);
                self.dse[self.psw.reg_set as usize][(r1 & 7) as usize] = (cv & 0xF) as u8;
            }
            Stxar | Stxa => {
                // §9.14: store R1's extended address as a fullword address
                // constant: bit 0 set, bits 1-15 from R1, bits 16-19
                // zeroed, bits 20-27 of the destination unchanged, bits
                // 28-31 from R1's DSE.
                let dse =
                    self.dse[self.psw.reg_set as usize][(r1 & 7) as usize] as u32;
                let make = |old: u32, r1v: u32| {
                    0x8000_0000 | (r1v & 0x7FFF_0000) | (old & 0x0000_0FF0) | dse
                };
                match dec.operand {
                    Operand::R(r2) => {
                        let v = make(self.r(r2), self.r(r1));
                        self.set_r(r2, v);
                    }
                    _ => {
                        let addr = self.storage_addr(dec, at)?;
                        let old = self.mem.read_f(addr).map_err(|e| trap_addr(e, at))?;
                        let v = make(old, self.r(r1));
                        self.store_f_prot(addr, v, at)?;
                    }
                }
            }
            Ldm => {
                // §9.13: DSEs for R0-R3 of the current set from the four
                // low nibbles of the operand's bytes.
                let addr = self.storage_addr(dec, at)?;
                let v = self.mem.read_f(addr).map_err(|e| trap_addr(e, at))?;
                for n in 0..4usize {
                    self.dse[self.psw.reg_set as usize][n] =
                        ((v >> (24 - 8 * n)) & 0xF) as u8;
                }
            }
            Pc => {
                // §3.3: privileged; the control word is in R2 (bit 0:
                // 0 = input, 1 = output) and the data fullword in R1.
                // CC 00 = successful, 01 = interface timeout. Note:
                // yaGPC2/nsts-sim-gpc read the CW from R1 and data from
                // R2, contradicting the PoO text; the PoO is followed
                // here (see ISA_STATUS.md).
                if self.privileged_violation(at)? {
                    return Ok(());
                }
                let r2 = operand_reg(dec);
                let cw = self.r(r2);
                let data = if cw & 0x8000_0000 != 0 {
                    Some(self.r(r1))
                } else {
                    None
                };
                let response = match self.io.as_mut() {
                    Some(io) => io.pc(cw, data),
                    None => PcResponse::Timeout,
                };
                match response {
                    PcResponse::Input(v) => {
                        self.set_r(r1, v);
                        self.psw.cc = cc::ZERO;
                    }
                    PcResponse::OutputAccepted => self.psw.cc = cc::ZERO,
                    PcResponse::Timeout => self.psw.cc = cc::POS,
                }
            }
            Stdm => {
                // §9.15: store the DSEs for R0-R3.
                let addr = self.storage_addr(dec, at)?;
                let set = self.psw.reg_set as usize;
                let v = ((self.dse[set][0] as u32) << 24)
                    | ((self.dse[set][1] as u32) << 16)
                    | ((self.dse[set][2] as u32) << 8)
                    | self.dse[set][3] as u32;
                self.store_f_prot(addr, v, at)?;
            }
        }
        Ok(())
    }

    // ----- floating point helpers -----

    fn fpr_short(&self, n: u8) -> Unpacked {
        float::unpack_short(self.fpr[(n & 7) as usize])
    }

    fn fpr_long(&self, n: u8) -> Unpacked {
        float::unpack_long(
            self.fpr[(n & 7) as usize],
            self.fpr[((n + 1) % 8) as usize],
        )
    }

    /// Short second operand: FPR (RR forms) or a storage fullword.
    fn fp_operand_short(&mut self, dec: &Decoded, at: u32) -> Result<Unpacked, Trap> {
        match dec.operand {
            Operand::R(r2) => Ok(self.fpr_short(r2)),
            _ => {
                let addr = self.storage_addr(dec, at)?;
                let w = self.mem.read_f(addr).map_err(|e| trap_addr(e, at))?;
                Ok(float::unpack_short(w))
            }
        }
    }

    /// Long second operand: FPR pair (RR forms) or a storage doubleword.
    fn fp_operand_long(&mut self, dec: &Decoded, at: u32) -> Result<Unpacked, Trap> {
        match dec.operand {
            Operand::R(r2) => Ok(self.fpr_long(r2)),
            _ => {
                let addr = self.storage_addr(dec, at)?;
                let hi = self.mem.read_f(addr).map_err(|e| trap_addr(e, at))?;
                let lo = self.mem.read_f(addr + 2).map_err(|e| trap_addr(e, at))?;
                Ok(float::unpack_long(hi, lo))
            }
        }
    }

    /// CC for FP results: 00 zero fraction, 11 negative, 01 positive (§8.7).
    fn fp_cc_value(&mut self, u: Unpacked) {
        self.psw.cc = if u.is_zero() {
            cc::ZERO
        } else if u.neg {
            cc::NEG
        } else {
            cc::POS
        };
    }

    fn fp_cc_compare(&mut self, ord: i32) {
        self.psw.cc = match ord {
            0 => cc::ZERO,
            o if o < 0 => cc::NEG,
            _ => cc::POS,
        };
    }

    /// Complete a short-precision FP operation: write the result and set
    /// the CC (if `sets_cc`), honoring the §8.8 exception rules.
    fn fp_finish_short(
        &mut self,
        r1: u8,
        r: Unpacked,
        ev: FpEvent,
        sets_cc: bool,
        at: u32,
    ) -> Result<(), Trap> {
        match ev {
            FpEvent::None => {
                self.fpr[(r1 & 7) as usize] = float::pack_short(r);
                if sets_cc {
                    self.fp_cc_value(r);
                }
                Ok(())
            }
            FpEvent::Overflow => self.program_interrupt(pe_code::FP_OVERFLOW, at),
            FpEvent::Underflow => {
                if self.psw.exp_underflow_mask {
                    self.program_interrupt(pe_code::FP_UNDERFLOW, at)
                } else {
                    self.fpr[(r1 & 7) as usize] = 0;
                    if sets_cc {
                        self.psw.cc = cc::ZERO;
                    }
                    Ok(())
                }
            }
            FpEvent::Significance => {
                // True zero written regardless; interrupt if masked on.
                self.fpr[(r1 & 7) as usize] = 0;
                if sets_cc {
                    self.psw.cc = cc::ZERO;
                }
                if self.psw.significance_mask {
                    self.program_interrupt(pe_code::SIGNIFICANCE, at)
                } else {
                    Ok(())
                }
            }
            FpEvent::DivideException => self.program_interrupt(pe_code::FP_DIVIDE, at),
        }
    }

    fn fp_finish_long(
        &mut self,
        r1: u8,
        r: Unpacked,
        ev: FpEvent,
        sets_cc: bool,
        at: u32,
    ) -> Result<(), Trap> {
        let write = |cpu: &mut Cpu, u: Unpacked| {
            let (hi, lo) = float::pack_long(u);
            cpu.fpr[(r1 & 7) as usize] = hi;
            cpu.fpr[((r1 + 1) % 8) as usize] = lo;
        };
        match ev {
            FpEvent::None => {
                write(self, r);
                if sets_cc {
                    self.fp_cc_value(r);
                }
                Ok(())
            }
            FpEvent::Overflow => self.program_interrupt(pe_code::FP_OVERFLOW, at),
            FpEvent::Underflow => {
                if self.psw.exp_underflow_mask {
                    self.program_interrupt(pe_code::FP_UNDERFLOW, at)
                } else {
                    write(self, float::TRUE_ZERO);
                    if sets_cc {
                        self.psw.cc = cc::ZERO;
                    }
                    Ok(())
                }
            }
            FpEvent::Significance => {
                write(self, float::TRUE_ZERO);
                if sets_cc {
                    self.psw.cc = cc::ZERO;
                }
                if self.psw.significance_mask {
                    self.program_interrupt(pe_code::SIGNIFICANCE, at)
                } else {
                    Ok(())
                }
            }
            FpEvent::DivideException => self.program_interrupt(pe_code::FP_DIVIDE, at),
        }
    }

    /// Privileged-instruction check (§2.3, §2.5.4.1): in problem state the
    /// attempt produces a program interrupt with code 0001 and the
    /// instruction is not executed. Returns true when the interrupt was
    /// taken (caller must skip the instruction body).
    fn privileged_violation(&mut self, at: u32) -> Result<bool, Trap> {
        if self.psw.problem_state {
            self.program_interrupt(pe_code::PRIVILEGED, at)?;
            Ok(true)
        } else {
            Ok(false)
        }
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
    fn multiply_frac32(&mut self, r1: u8, op: u32) {
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
    }

    /// Fractional halfword multiply (§4.22/4.23): 32-bit product fraction
    /// of two halfword fractions, into all of R1.
    fn multiply_frac16(&mut self, r1: u8, b: u16) {
        let a = (self.r(r1) >> 16) as u16 as i16 as i32;
        let b = b as i16 as i32;
        let p = ((a * b) as u32) << 1;
        let ovf = a == i16::MIN as i32 && b == i16::MIN as i32;
        if ovf {
            self.psw.overflow = true;
        }
        self.set_r(r1, p);
    }

    /// Fractional divide (§4.10): 64-bit dividend in R1:(R1+1)mod8 (odd R1:
    /// R1 with 32 low-order zeros appended); unrounded quotient to R1.
    /// Overflow (quotient unrepresentable or divide by zero) leaves the
    /// registers unchanged — the manual calls them indeterminate; this
    /// emulator's deterministic choice is documented in ISA_STATUS.md.
    fn divide_frac(&mut self, r1: u8, divisor: i32) {
        let dividend: i64 = if r1 % 2 == 1 {
            (self.r(r1) as i64) << 32
        } else {
            (((self.r(r1) as u64) << 32) | self.r((r1 + 1) % 8) as u64) as i64
        };
        if divisor == 0 {
            self.psw.overflow = true;
            return;
        }
        let q = dividend as i128 / (divisor as i128 * 2);
        if q < i32::MIN as i128 || q > i32::MAX as i128 {
            self.psw.overflow = true;
            return;
        }
        self.set_r(r1, q as i32 as u32);
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

/// MVH destination offset: bits 1-15 of R1 (§9.4).
fn r2v_dest(r1v: u32) -> u16 {
    ((r1v >> 16) & 0x7FFF) as u16
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
