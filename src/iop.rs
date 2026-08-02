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
use std::collections::VecDeque;

/// One word on a serial data bus (App. III §1.1.1): 28 bits on the wire —
/// 3 sync, 24 information, 1 parity. Parity is modeled as always good;
/// the sync type distinguishes commands from data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusWord {
    pub cmd_sync: bool,
    /// The 24 information bits (wire bits 3-26).
    pub info: u32,
}

impl BusWord {
    /// Data word from a BCE (Figure 1.1(a)): IUA in info bits 0-4, 16
    /// data bits in 8-23 (info bits 5-20), SEV pattern 101 in the tail.
    pub fn data(iua: u8, data: u16) -> BusWord {
        BusWord {
            cmd_sync: false,
            info: ((iua as u32 & 0x1F) << 19) | ((data as u32) << 3) | 0b101,
        }
    }

    pub fn command(cmd24: u32) -> BusWord {
        BusWord { cmd_sync: true, info: cmd24 & 0x00FF_FFFF }
    }

    /// Listen command (Figure 1.1(d)): common IOP address 01000 in info
    /// bits 0-4, target IUA in info bits 11-15, branch-table index in
    /// info bits 16-23 (§1.1.1, #WIX p. III-38).
    pub fn listen(iua: u8, index: u8) -> BusWord {
        BusWord {
            cmd_sync: true,
            info: (0b01000 << 19) | ((iua as u32 & 0x1F) << 8) | index as u32,
        }
    }
}

/// The serial-bus fabric a GPC's 24 MIAs attach to. This is the seam for
/// the multi-GPC redundant set: a shared fabric routes one GPC's
/// transmissions into other GPCs' receive queues (listen mode, App. III
/// §4). `LocalBuses` is the single-GPC implementation.
pub trait BusFabric {
    /// Put a word on bus `bus` (0-23) from this GPC's transmitter.
    fn transmit(&mut self, bus: usize, w: BusWord);
    /// Next word available to this GPC's receiver on bus `bus`.
    fn receive(&mut self, bus: usize) -> Option<BusWord>;
}

/// Single-GPC fabric: records transmissions, delivers injected words.
pub struct LocalBuses {
    pub sent: Vec<(usize, BusWord)>,
    pub rx: Vec<VecDeque<BusWord>>,
}

impl Default for LocalBuses {
    fn default() -> LocalBuses {
        LocalBuses { sent: Vec::new(), rx: (0..NUM_BCES).map(|_| VecDeque::new()).collect() }
    }
}

impl LocalBuses {
    pub fn new() -> LocalBuses {
        LocalBuses::default()
    }

    pub fn inject(&mut self, bus: usize, w: BusWord) {
        self.rx[bus].push_back(w);
    }
}

impl BusFabric for LocalBuses {
    fn transmit(&mut self, bus: usize, w: BusWord) {
        self.sent.push((bus, w));
    }

    fn receive(&mut self, bus: usize) -> Option<BusWord> {
        self.rx[bus].pop_front()
    }
}

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
    /// Maximum Time Out register (§1.2.1); units of 16.5 µs on the
    /// hardware. Timing is not modeled: an empty receive queue counts as
    /// an immediate timeout.
    pub mto: u32,
    /// Interface Unit Address register (5 bits, §1.2.1): the subsystem
    /// this BCE last commanded; receive checks inputs against it.
    pub iuar: u8,
    /// BCE status register (Table 1.2, `bce_status` bit constants).
    pub status: u32,
    /// Program-exception state (STAT1): true = an error was recorded.
    pub error: bool,
    /// An in-progress receive (§3.4): reception is asynchronous on the
    /// hardware — the BCE waits up to MTO (units of 16.5 µs; one step
    /// here) for each input word. The instruction stays current until
    /// the stream completes or times out.
    pub rx: Option<RxState>,
}

#[derive(Debug, Clone)]
pub struct RxState {
    remaining: u32,
    addr: u32,
    /// Instruction length to add to the PC on completion.
    ilen: u32,
    /// Steps left before a timeout is declared.
    timer: u32,
    /// Whether any word of the stream has arrived (initial vs ordinary
    /// timeout, Table 1.2 bits 25/26).
    got_any: bool,
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

/// MSC status-word error bits, positioned as @LMS loads them into the
/// ACC (App. II p. II-57: bit 31 = busy, 30 = program exception,
/// 29 illegal opcode, 28 boundary alignment, 27 LBB error, 26 LBP
/// error, 25 SIO error; IBM bit 31 = LSB).
pub mod msc_status {
    pub const BUSY: u32 = 1 << 0;
    pub const PROGRAM_EXCEPTION: u32 = 1 << 1;
    pub const ILLEGAL: u32 = 1 << 2;
    pub const BOUNDARY: u32 = 1 << 3;
    pub const LBB_ERR: u32 = 1 << 4;
    pub const LBP_ERR: u32 = 1 << 5;
    pub const SIO_ERR: u32 = 1 << 6;
}

/// BCE status-register bits (App. III Table 1.2, IBM bit b = value bit
/// 31-b). The parity/signature/SEV low-field positions in the scan are
/// partially illegible; the choices here are recorded in IOP_STATUS.md.
pub mod bce_status {
    pub const ILLEGAL: u32 = 1 << 2; // IBM bit 29
    pub const BOUNDARY: u32 = 1 << 3; // 28
    pub const BLOCK_TIMEOUT: u32 = 1 << 4; // 27
    pub const TIMEOUT: u32 = 1 << 5; // 26
    pub const INITIAL_TIMEOUT: u32 = 1 << 6; // 25
    pub const XMT_DISABLED: u32 = 1 << 8; // 23
    pub const GAP: u32 = 1 << 10; // 21
    pub const SYNC_ERROR: u32 = 1 << 16; // 15
    /// SEV-pattern field (IBM bits 9-11).
    pub const SEV: u32 = 0b111 << 20;
    /// Signature (IUA) mismatch flag; the offending IUA is also ORed
    /// into IBM bits 8-12.
    pub const SIG_MISMATCH: u32 = 1 << 24;
}

pub struct Iop {
    pub msc: Msc,
    pub bces: Vec<Bce>,
    /// Interrupt registers A-E (App. I "READ INTERRUPT REG." commands).
    pub interrupt_regs: [u32; 5],
    /// MIA transmitter/receiver enable registers: IBM bit i = BCE/MIA i
    /// (App. III §1.2.2; changed only by CPU PCOs). The PCO data-word
    /// format page is thin — the data word is taken as the new register
    /// value (PARTIAL, see IOP_STATUS.md).
    pub mia_xmtr_enable: u32,
    pub mia_rcvr_enable: u32,
    /// Discrete output register.
    pub discrete_out: bool,
    /// Discrete inputs (PCI "D.I.A (1-32)", App. I p. I-5): lines driven
    /// by external equipment — in the redundant set, the other GPCs'
    /// discrete outputs (the sync-discrete arrangement). IBM bit i =
    /// line i.
    pub discrete_in: u32,
    /// PROCESSOR HALT / PROCESSOR ENABLE state (App. I p. I-4): halted
    /// processors do not sequence.
    pub halted: bool,
    /// ENABLE INTERRUPTS state.
    pub interrupts_enabled: bool,
    /// MSC status error bits (`msc_status`).
    pub msc_errors: u32,
    /// Fail-discrete register (App. II p. II-54: IBM bits 0-8, i.e. the
    /// top bits of the word).
    pub fail_discretes: u32,
    /// BCE-MSC indicator register: BCE i at IBM bit i.
    pub indicators: u32,
    /// MSC Local Store register C6 (external-call mailbox, App. II
    /// p. II-62): the CPU writes a program address here; @SEC samples it.
    pub c6: u32,
    /// Raised by the MSC @INT instruction: the 11-bit level field, for
    /// the host to route to a CPU external interrupt. PARTIAL: the level
    /// field's encoding page (App. II p. II-94) is thin; the raw field is
    /// exposed unchanged.
    pub cpu_interrupt: Option<u16>,
}

impl Default for Iop {
    fn default() -> Iop {
        Iop {
            msc: Msc::default(),
            bces: vec![Bce::default(); NUM_BCES],
            interrupt_regs: [0; 5],
            mia_xmtr_enable: 0,
            mia_rcvr_enable: 0,
            discrete_out: false,
            discrete_in: 0,
            halted: true,
            interrupts_enabled: false,
            msc_errors: 0,
            fail_discretes: 0,
            indicators: 0,
            c6: 0,
            cpu_interrupt: None,
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

    /// Advance the IOP one time slice: the MSC executes one instruction
    /// (if busy) and every busy BCE executes one instruction. This
    /// mirrors the time-shared implementation (App. III §1.3: each
    /// processor gets a microinstruction slot every 16.5 µs). No-op when
    /// PROCESSOR HALT is in effect.
    pub fn step(&mut self, mem: &mut crate::mem::Memory, buses: &mut dyn BusFabric) {
        if self.halted {
            return;
        }
        if self.msc.busy {
            self.step_msc(mem);
        }
        for n in 0..NUM_BCES {
            if self.bces[n].busy {
                self.step_bce(n, mem, buses);
            }
        }
    }

    fn xmtr_enabled(&self, n: usize) -> bool {
        self.mia_xmtr_enable & (1 << (31 - n)) != 0
    }

    fn rcvr_enabled(&self, n: usize) -> bool {
        self.mia_rcvr_enable & (1 << (31 - n)) != 0
    }

    /// Error-terminate a BCE instruction (App. III §2/#RDS pages): status
    /// bits recorded, program-exception state set, BCE-MSC indicator
    /// raised, Wait State entered.
    fn bce_error(&mut self, n: usize, bits: u32) {
        let b = &mut self.bces[n];
        b.rx = None;
        b.status |= bits;
        b.error = true;
        b.indicator = true;
        b.busy = false;
        self.indicators |= 1 << (31 - (n as u32 + 1));
    }

    /// One BCE instruction (App. III §3). `n` is the zero-based index;
    /// the architectural BCE number (used in direct-mode table indexing,
    /// p. III-11) is n+1.
    fn step_bce(&mut self, n: usize, mem: &mut crate::mem::Memory, buses: &mut dyn BusFabric) {
        let num = n as u32 + 1;
        // Resume an in-progress receive without re-decoding (a #MIN's
        // command must not be retransmitted on resume).
        if self.bces[n].rx.is_some() {
            self.receive_pump(n, mem, buses);
            return;
        }
        let pc = self.bces[n].pc & 0x3FFFF;
        let hw1 = mem.read_h(pc).unwrap_or(0) as u32;
        let hw2 = mem.read_h(pc.wrapping_add(1) & 0x3FFFF).unwrap_or(0) as u32;
        let op4 = hw1 >> 12;
        let top5 = hw1 >> 11;
        let disp11 = hw1 & 0x7FF;

        // Long-format instructions must sit on fullword boundaries
        // (Table 1.2 boundary-alignment error).
        if op4 == 0xF && pc & 1 != 0 {
            self.bce_error(n, bce_status::BOUNDARY);
            return;
        }

        match () {
            // #WAT (§3.5): enter the Wait State.
            _ if top5 == 0b00001 => {
                self.bces[n].pc = pc.wrapping_add(1) & 0x3FFFF;
                self.bces[n].busy = false;
            }
            // #STP (§3.5): stop.
            _ if op4 == 0b0001 => {
                self.bces[n].pc = pc.wrapping_add(1) & 0x3FFFF;
                self.bces[n].busy = false;
            }
            // #WIX (p. III-38): in listen mode (transmitter disabled),
            // wait for a Listen Command and branch through the table at
            // updated-PC + disp (rounded to even). With the transmitter
            // enabled, behaves as #WAT.
            _ if top5 == 0b00100 => {
                if self.xmtr_enabled(n) {
                    self.bces[n].pc = pc.wrapping_add(1) & 0x3FFFF;
                    self.bces[n].busy = false;
                    return;
                }
                if !self.rcvr_enabled(n) {
                    return; // nothing can arrive; keep monitoring
                }
                match buses.receive(n) {
                    Some(w) if w.cmd_sync && w.info >> 19 == 0b01000 => {
                        let iua = ((w.info >> 8) & 0x1F) as u8;
                        let index = w.info & 0xFF;
                        let table = (pc.wrapping_add(1).wrapping_add(disp11) + 1) & !1;
                        let target = mem
                            .read_f((table + 2 * index) & 0x3FFFF)
                            .unwrap_or(0);
                        self.bces[n].iuar = iua;
                        self.bces[n].pc = target & 0x3FFFF;
                    }
                    // Not a listen command, or nothing yet: keep looping
                    // at this instruction (the hardware's tight loop).
                    _ => {}
                }
            }
            // #SSC / #SST (p. III-33/34): store status (M bit indexes by
            // twice the BCE number); #SSC then clears it.
            _ if op4 == 0b0100 || op4 == 0b0101 => {
                let m = hw1 & 0x0800 != 0;
                let d = if disp11 & 0x400 != 0 { disp11 | !0x7FF } else { disp11 };
                let mut ea = pc.wrapping_add(1).wrapping_add(d) & 0x3FFFF;
                if m {
                    ea = ea.wrapping_add(2 * num) & 0x3FFFF;
                }
                let st = self.bces[n].status;
                let _ = mem.write_f(ea & !1, st);
                self.bces[n].pc = pc.wrapping_add(1) & 0x3FFFF;
                if op4 == 0b0100 {
                    self.bces[n].status = 0;
                    self.bces[n].error = false;
                }
            }
            // #TDS (p. III-51): transmit count+1 halfwords from
            // base+disp8.
            _ if hw1 >> 13 == 0b100 && top5 != 0b10110 && top5 != 0b10111 => {
                let count = ((hw1 >> 8) & 0x1F) + 1;
                let start = self.bces[n].base.wrapping_add(hw1 & 0xFF) & 0x3FFFF;
                self.transmit_data(n, mem, buses, start, count, 1);
            }
            // #RDS (p. III-71): receive count+1 halfwords into
            // base+disp8.
            _ if hw1 >> 13 == 0b011 => {
                let count = ((hw1 >> 8) & 0x1F) + 1;
                let start = self.bces[n].base.wrapping_add(hw1 & 0xFF) & 0x3FFFF;
                self.receive_data(n, mem, buses, start, count, 1);
            }
            // #LTOI / #LTO (p. III-30): load the MTO register.
            _ if top5 == 0b10110 => {
                self.bces[n].mto = disp11;
                self.bces[n].pc = pc.wrapping_add(1) & 0x3FFFF;
            }
            _ if top5 == 0b10111 => {
                let d = if disp11 & 0x400 != 0 { disp11 | !0x7FF } else { disp11 };
                let ea = pc
                    .wrapping_add(1)
                    .wrapping_add(d)
                    .wrapping_add(2 * num)
                    & 0x3FFFF;
                self.bces[n].mto = mem.read_f(ea & !1).unwrap_or(0) & 0x3FFFF;
                self.bces[n].pc = pc.wrapping_add(1) & 0x3FFFF;
            }
            // #DLYI / #DLY (§3.5): timed delays; timing unmodeled.
            _ if top5 == 0b11000 || top5 == 0b11001 => {
                self.bces[n].pc = pc.wrapping_add(1) & 0x3FFFF;
            }
            // #RIB / #SIB (p. III-31/32).
            _ if top5 == 0b11100 || top5 == 0b11101 => {
                let set = top5 == 0b11101;
                self.bces[n].indicator = set;
                let bit = 1u32 << (31 - num);
                if set {
                    self.indicators |= bit;
                } else {
                    self.indicators &= !bit;
                }
                self.bces[n].pc = pc.wrapping_add(1) & 0x3FFFF;
            }
            // Long formats.
            _ if op4 == 0xF => {
                let m = hw1 & 0x0800 != 0;
                let op3 = (hw1 >> 8) & 7;
                let val18 = ((hw1 & 3) << 16) | hw2;
                // Direct mode resolves through memory with automatic
                // indexing by twice the BCE number (p. III-11).
                let resolve = |mem: &crate::mem::Memory, v: u32| -> u32 {
                    mem.read_f((v.wrapping_add(2 * num) & 0x3FFFF) & !1).unwrap_or(0)
                };
                match op3 {
                    0b000 => {
                        // #BU
                        let t = if m { resolve(mem, val18) } else { val18 };
                        self.bces[n].pc = t & 0x3FFFF;
                    }
                    0b010 => {
                        // #LBR
                        let v = if m { resolve(mem, val18) } else { val18 };
                        self.bces[n].base = v & 0x3FFFF;
                        self.bces[n].pc = pc.wrapping_add(2) & 0x3FFFF;
                    }
                    0b110 => {
                        // #CMDI / #CMD (p. III-49): transmit a 24-bit
                        // command; IUAR gets its top 5 bits. Disabled
                        // transmitter or busy MIA: no action, no error.
                        let cmd = if m {
                            resolve(mem, val18) & 0x00FF_FFFF
                        } else {
                            (((hw1 >> 3) & 0x1F) << 19) | ((hw1 & 7) << 16) | hw2
                        };
                        if !m || self.xmtr_enabled(n) {
                            self.bces[n].iuar = (cmd >> 19) as u8 & 0x1F;
                        }
                        if self.xmtr_enabled(n) {
                            buses.transmit(n, BusWord::command(cmd));
                        }
                        self.bces[n].pc = pc.wrapping_add(2) & 0x3FFFF;
                    }
                    0b100 => {
                        // #TDLI / #TDL (p. III-53): long transmit from
                        // the base.
                        let count =
                            (if m { resolve(mem, val18) } else { val18 } & 0x3FFFF) + 1;
                        let start = self.bces[n].base & 0x3FFFF;
                        self.transmit_data(n, mem, buses, start, count, 2);
                    }
                    0b011 => {
                        // #RDLI / #RDL (p. III-73): long receive at the
                        // base.
                        let count =
                            (if m { resolve(mem, val18) } else { val18 } & 0x3FFFF) + 1;
                        let start = self.bces[n].base & 0x3FFFF;
                        self.receive_data(n, mem, buses, start, count, 2);
                    }
                    0b101 | 0b001 => {
                        // #MOUT / #MIN (p. III-54/III-74): command +
                        // data stream in one instruction. Two-fullword
                        // form: displacement (8) + transfer count (16)
                        // in word one, the 24-bit command at PC+2.
                        // Indexed form (M=1): per-BCE tables at
                        // ADDRESS+2i and ADDRESS+48+2i.
                        let (disp, tc, cmd) = if m {
                            let e1 = resolve(mem, val18);
                            let cmd = mem
                                .read_f((val18 + 48 + 2 * num) & 0x3FFFF & !1)
                                .unwrap_or(0);
                            (e1 >> 16 & 0x7FF, e1 & 0xFFFF, cmd & 0x00FF_FFFF)
                        } else {
                            let cmd = mem
                                .read_f(pc.wrapping_add(2) & 0x3FFFF)
                                .unwrap_or(0);
                            (hw1 & 0xFF, hw2, cmd & 0x00FF_FFFF)
                        };
                        if !self.xmtr_enabled(n) {
                            self.bce_error(n, bce_status::XMT_DISABLED);
                            return;
                        }
                        self.bces[n].iuar = (cmd >> 19) as u8 & 0x1F;
                        buses.transmit(n, BusWord::command(cmd));
                        let start =
                            (self.bces[n].base.wrapping_add(disp) & !1) & 0x3FFFF;
                        let len = if m { 2 } else { 4 };
                        if op3 == 0b101 {
                            self.transmit_data(n, mem, buses, start, tc + 1, len);
                        } else {
                            self.receive_data(n, mem, buses, start, tc + 1, len);
                        }
                    }
                    _ => {
                        self.bce_error(n, bce_status::ILLEGAL);
                    }
                }
            }
            _ => {
                self.bce_error(n, bce_status::ILLEGAL);
            }
        }
    }

    /// Common transmit path (#TDS/#TDL/#MOUT data phase): `count`
    /// halfwords from `start`, then PC += `ilen`. A disabled transmitter
    /// is the Table 1.2 bit-23 error.
    fn transmit_data(
        &mut self,
        n: usize,
        mem: &crate::mem::Memory,
        buses: &mut dyn BusFabric,
        start: u32,
        count: u32,
        ilen: u32,
    ) {
        if !self.xmtr_enabled(n) {
            self.bce_error(n, bce_status::XMT_DISABLED);
            return;
        }
        let iua = self.bces[n].iuar;
        for i in 0..count {
            let hw = mem.read_h((start + i) & 0x3FFFF).unwrap_or(0);
            buses.transmit(n, BusWord::data(iua, hw));
        }
        let b = &mut self.bces[n];
        b.pc = (b.pc + ilen) & 0x3FFFF;
    }

    /// Begin a receive (#RDS/#RDL/#MIN data phase): `count` halfwords
    /// into `start`. The BCE then pumps the stream one time slice at a
    /// time, waiting up to the MTO register's count (one step per 16.5
    /// µs unit) for each word — first-word latency and interword gaps
    /// per §3.4.1-3.4.8.
    fn receive_data(
        &mut self,
        n: usize,
        mem: &mut crate::mem::Memory,
        buses: &mut dyn BusFabric,
        start: u32,
        count: u32,
        ilen: u32,
    ) {
        let timer = self.bces[n].mto;
        self.bces[n].rx = Some(RxState {
            remaining: count,
            addr: start,
            ilen,
            timer,
            got_any: false,
        });
        self.receive_pump(n, mem, buses);
    }

    /// One time slice of an in-progress receive: drain available words
    /// (validity-checked per Table 1.2), or burn one timeout tick.
    fn receive_pump(
        &mut self,
        n: usize,
        mem: &mut crate::mem::Memory,
        buses: &mut dyn BusFabric,
    ) {
        let mut st = self.bces[n].rx.take().expect("receive in progress");
        loop {
            if st.remaining == 0 {
                let b = &mut self.bces[n];
                b.pc = (b.pc + st.ilen) & 0x3FFFF;
                return;
            }
            let w = if self.rcvr_enabled(n) { buses.receive(n) } else { None };
            let w = match w {
                Some(w) => w,
                None => {
                    if st.timer == 0 {
                        self.bce_error(
                            n,
                            if st.got_any {
                                bce_status::TIMEOUT
                            } else {
                                bce_status::INITIAL_TIMEOUT
                            },
                        );
                    } else {
                        st.timer -= 1;
                        self.bces[n].rx = Some(st);
                    }
                    return;
                }
            };
            if w.cmd_sync {
                // Listen mode accepts a command as the first input — the
                // commander's own command passing by (§3.4.3, Figure 3.8
                // listen-mode column); command sync mid-stream is the
                // Table 1.2 sync error.
                if !st.got_any && !self.xmtr_enabled(n) {
                    continue;
                }
                self.bce_error(n, bce_status::SYNC_ERROR);
                return;
            }
            let iua = (w.info >> 19) as u8 & 0x1F;
            let sev = w.info & 7;
            if iua != self.bces[n].iuar {
                self.bce_error(
                    n,
                    bce_status::SIG_MISMATCH | ((iua as u32) << 19),
                );
                return;
            }
            if sev != 0b101 {
                self.bce_error(n, (sev ^ 0b101) << 20);
                return;
            }
            let data = (w.info >> 3) as u16;
            let _ = mem.write_h(st.addr & 0x3FFFF, data);
            st.remaining -= 1;
            st.addr += 1;
            st.got_any = true;
            st.timer = self.bces[n].mto;
        }
    }

    fn err(&mut self, bit: u32) {
        self.msc_errors |= bit | msc_status::PROGRAM_EXCEPTION;
    }

    /// Short-format effective address (App. II §1.1/yaGPC2 iop_msc_ea):
    /// PC-relative signed 11-bit displacement, optionally indexed by X;
    /// 18-bit wrap.
    fn short_ea(&self, disp11: u32, indexed: bool) -> u32 {
        let d = if disp11 & 0x400 != 0 { disp11 | !0x7FF } else { disp11 };
        let mut ea = self.msc.pc.wrapping_add(d) & 0x3FFFF;
        if indexed {
            ea = ea.wrapping_add(self.msc.x) & 0x3FFFF;
        }
        ea
    }

    /// Long-format effective value (App. II Table 1.2): M=0 immediate
    /// (18-bit, sign-extended when used as data), M=1 direct (fullword at
    /// the address); both optionally indexed.
    fn long_ev(&self, mem: &crate::mem::Memory, addr18: u32, i: bool, m: bool) -> u32 {
        let mut a = addr18 & 0x3FFFF;
        if i {
            a = a.wrapping_add(self.msc.x) & 0x3FFFF;
        }
        if m {
            mem.read_f(a & !1).unwrap_or(0)
        } else {
            // sign-extend 18 -> 32 for immediate use
            if a & 0x20000 != 0 { a | !0x3FFFF } else { a }
        }
    }

    fn read_f(&self, mem: &crate::mem::Memory, ea: u32) -> u32 {
        mem.read_f(ea & !1 & 0x3FFFF).unwrap_or(0)
    }

    fn write_f(&mut self, mem: &mut crate::mem::Memory, ea: u32, v: u32) {
        let _ = mem.write_f(ea & !1 & 0x3FFFF, v);
    }

    /// The register @LAR selects (App. II p. II-54).
    fn lar_register(&self, reg: u32) -> u32 {
        match reg & 3 {
            0 => {
                // STAT1: program-exception indicators; MSC = bit 0.
                if self.msc_errors & msc_status::PROGRAM_EXCEPTION != 0 {
                    0x8000_0000
                } else {
                    0
                }
            }
            1 => self.indicators,
            2 => self.fail_discretes,
            _ => self.busy_wait_register(),
        }
    }

    fn step_msc(&mut self, mem: &mut crate::mem::Memory) {
        let pc = self.msc.pc & 0x3FFFF;
        let hw1 = mem.read_h(pc).unwrap_or(0) as u32;
        let op = hw1 >> 12;
        let ibit = hw1 & 0x0800 != 0;
        let disp11 = hw1 & 0x7FF;
        let imm8 = {
            let d = hw1 & 0xFF;
            if d & 0x80 != 0 { (d | !0xFF) as i32 } else { d as i32 }
        };
        // long format pieces
        let hw2 = mem.read_h(pc.wrapping_add(1) & 0x3FFFF).unwrap_or(0) as u32;
        let op3 = (hw1 >> 8) & 7;
        let f5 = (hw1 >> 3) & 0x1F; // bits 8-12
        let mbit = hw1 & 0x0004 != 0; // bit 13
        let addr18 = ((hw1 & 3) << 16) | hw2;
        let acc = self.msc.acc as i32;

        // Long-format instructions must sit on even halfword boundaries
        // (App. II p. II-6): boundary-alignment error terminates.
        if op == 0xF && pc & 1 != 0 {
            self.err(msc_status::BOUNDARY);
            self.msc.busy = false;
            return;
        }

        match op {
            0x0 if hw1 == 0x0800 => {
                // @WAT: enter the wait state (App. II p. II-92).
                self.msc.busy = false;
            }
            0x1 if hw1 == 0x1000 => {
                // @STP: stop the MSC (App. II p. II-95).
                self.msc.busy = false;
            }
            0x2 => {
                // @BC tests the ACC; @BXC (I bit set) tests X
                // sign-extended (p. II-38/II-39). Condition bits 5-7:
                // =0 / <0 / >0.
                let v = if ibit { sign18(self.msc.x) } else { acc };
                let cond = (hw1 >> 8) & 7;
                let hit = (cond & 4 != 0 && v == 0)
                    || (cond & 2 != 0 && v < 0)
                    || (cond & 1 != 0 && v > 0);
                self.msc.pc = self.msc.pc.wrapping_add(1) & 0x3FFFF;
                if hit {
                    self.msc.pc = self.msc.pc.wrapping_add(imm8 as u32) & 0x3FFFF;
                }
                return;
            }
            0x3 => {
                // @INT: request a CPU interrupt; raw 11-bit level field
                // exposed to the host (see field doc).
                self.cpu_interrupt = Some((hw1 & 0xFFF) as u16);
            }
            0x4 => {
                let v = self.read_f(mem, self.short_ea(disp11, ibit));
                self.msc.acc = v;
            }
            0x5 => {
                let v = self.read_f(mem, self.short_ea(disp11, ibit));
                self.msc.acc = self.msc.acc.wrapping_add(v);
            }
            0x6 => {
                let v = self.read_f(mem, self.short_ea(disp11, ibit));
                self.msc.acc &= v;
            }
            0x7 => {
                let v = self.read_f(mem, self.short_ea(disp11, ibit));
                self.msc.acc ^= v;
            }
            0x8 => {
                let ea = self.short_ea(disp11, ibit);
                let a = self.msc.acc;
                self.write_f(mem, ea, a);
            }
            0x9 => {
                // @TSZ: increment the fullword; skip next instruction on
                // zero (p. II-46).
                let ea = self.short_ea(disp11, ibit);
                let v = self.read_f(mem, ea).wrapping_add(1);
                self.write_f(mem, ea, v);
                self.msc.pc =
                    self.msc.pc.wrapping_add(if v == 0 { 2 } else { 1 }) & 0x3FFFF;
                return;
            }
            0xA => {
                // @REC: reload Status/ACC/X/PC from 4 fullwords (p. II-43).
                let ea = self.short_ea(disp11, ibit) & !1;
                let stat = self.read_f(mem, ea);
                self.msc.acc = self.read_f(mem, ea + 2);
                self.msc.x = self.read_f(mem, ea + 4) & 0x3FFFF;
                self.msc.pc = self.read_f(mem, ea + 6) & 0x3FFFF;
                self.msc_errors = stat & 0x3FFFF & !msc_status::BUSY;
                self.msc.busy = stat & msc_status::BUSY != 0 || self.msc.busy;
                return;
            }
            0xC => {
                // @DLY: timed delay (p. II-93). Timing is not modeled; a
                // documented no-op.
            }
            0xD => {
                // Repeats (p. II-83..II-89): condition over BCEs selected
                // by ACC bits vs the indicator/busy-wait registers.
                // Without asynchronous BCE timing the condition is
                // evaluated once: met -> skip next halfword, else fall
                // through (the time-out path).
                let opx = (hw1 >> 8) & 7;
                let a = self.msc.acc;
                let met = match opx {
                    0b100 => a & !self.indicators == 0,            // @RAI all ind
                    0b000 => a & self.busy_wait_register() == 0,   // @RAW all wait
                    0b101 => a & self.indicators != 0,             // @RNI any ind
                    0b001 => a & !self.busy_wait_register() != 0,  // @RNW any wait
                    _ => {
                        self.err(msc_status::ILLEGAL);
                        false
                    }
                };
                self.msc.pc =
                    self.msc.pc.wrapping_add(if met { 2 } else { 1 }) & 0x3FFFF;
                return;
            }
            0xE if !ibit => {
                // Register operations (Table 3.1), OPX in bits 5-7.
                match (hw1 >> 8) & 7 {
                    0 => self.msc.acc = self.lar_register(hw1 & 0xFF), // @LAR
                    1 => self.fail_discretes |= self.msc.acc & 0xF800_0000, // @SFD
                    2 => self.fail_discretes &= !(self.msc.acc & 0xF800_0000), // @RFD
                    3 => {
                        // @LMS (p. II-57)
                        self.msc.acc = msc_status::BUSY | self.msc_errors;
                    }
                    4 => {
                        // @SIO: OR ACC into busy/wait (p. II-59). BCE i is
                        // IBM bit i; already-busy targets set the error.
                        let a = self.msc.acc;
                        if a & self.busy_wait_register() != 0 || a & 0x8000_0000 != 0 {
                            self.err(msc_status::SIO_ERR);
                        }
                        for n in 0..NUM_BCES {
                            if a & (1 << (30 - n)) != 0 {
                                self.bces[n].busy = true;
                            }
                        }
                    }
                    5 => {
                        // @XAX (p. II-61)
                        let x = self.msc.x;
                        self.msc.x = self.msc.acc & 0x3FFFF;
                        self.msc.acc =
                            if x & 0x20000 != 0 { x | !0x3FFFF } else { x };
                    }
                    6 => {
                        // @SEC (p. II-62): sample C6; if non-zero, save
                        // Status/ACC/X/PC+delta at (C6), branch to C6+8.
                        let c6 = self.c6;
                        if c6 != 0 {
                            let delta = hw1 & 0xFF;
                            let base = c6 & !1 & 0x3FFFF;
                            let stat = msc_status::BUSY | self.msc_errors;
                            let acc = self.msc.acc;
                            let x = self.msc.x;
                            let ret =
                                (self.msc.pc.wrapping_add(1).wrapping_add(delta))
                                    & 0x3FFFF;
                            self.write_f(mem, base, stat);
                            self.write_f(mem, base + 2, acc);
                            self.write_f(mem, base + 4, x);
                            self.write_f(mem, base + 6, ret);
                            self.msc.pc = c6.wrapping_add(8) & 0x3FFFF;
                            self.c6 = 0;
                            self.msc_errors = 0;
                            return;
                        }
                    }
                    _ => {
                        // @RBI (p. II-66): reset the BCE indicator; BCE
                        // number from bits 8-12 or ACC bits 27-31.
                        let mut n = f5;
                        if n == 0 {
                            n = self.msc.acc & 0x1F;
                        }
                        if (1..=24).contains(&n) {
                            self.indicators &= !(1 << (31 - n));
                        }
                    }
                }
            }
            0xE => {
                // Register immediates (Table 3.2), OPX in bits 5-7.
                match (hw1 >> 8) & 7 {
                    0 => {
                        // @NIX (p. II-69)
                        if self.msc.acc == 0 {
                            self.msc.x = 0;
                        } else {
                            while self.msc.acc & 0x8000_0000 == 0 {
                                self.msc.acc <<= 1;
                                self.msc.x =
                                    self.msc.x.wrapping_add(imm8 as u32) & 0x3FFFF;
                                if self.msc.acc == 0 {
                                    self.msc.x = 0;
                                    break;
                                }
                            }
                        }
                    }
                    1 => self.msc.x = self.msc.acc.wrapping_add(imm8 as u32) & 0x3FFFF, // @TAX
                    2 => {
                        // @TXI: X += imm; skip next when the result is
                        // non-positive. (The scan's "< 0" glyph is
                        // ambiguous; <= 0 per the yaGPC2 cross-check —
                        // see IOP_STATUS.md.)
                        let x = sign18(self.msc.x) + imm8;
                        self.msc.x = (x as u32) & 0x3FFFF;
                        self.msc.pc = self
                            .msc
                            .pc
                            .wrapping_add(if x <= 0 { 2 } else { 1 })
                            & 0x3FFFF;
                        return;
                    }
                    3 => self.msc.x = (imm8 as u32) & 0x3FFFF, // @LXI
                    4 => self.msc.acc = (sign18(self.msc.x) + imm8) as u32, // @TXA
                    5 => self.msc.acc = self.msc.acc.wrapping_add(imm8 as u32), // @TI
                    6 => self.msc.acc = (imm8 as u32).wrapping_sub(self.msc.acc), // @SAI
                    _ => self.msc.acc = imm8 as u32, // @LI
                }
            }
            0xF => {
                match op3 {
                    0 => {
                        // @BU (p. II-40)
                        self.msc.pc = self.long_ev(mem, addr18, ibit, mbit) & 0x3FFFF;
                        return;
                    }
                    1 => {
                        // @CALL (p. II-41): store PC+delta at EV, branch
                        // to EV+2.
                        let ev = self.long_ev(mem, addr18, ibit, mbit) & 0x3FFFF;
                        let ret = self.msc.pc.wrapping_add(f5) & 0x3FFFF;
                        self.write_f(mem, ev, ret);
                        self.msc.pc = ev.wrapping_add(2) & 0x3FFFF;
                        return;
                    }
                    2 | 3 => {
                        // @LBB/@LBP (p. II-50/II-51): BCE# from bits 8-12
                        // or ACC; only a waiting BCE is loaded.
                        let ev = self.long_ev(mem, addr18, ibit, mbit) & 0x3FFFF;
                        let mut n = f5;
                        if n == 0 {
                            n = self.msc.acc & 0x1F;
                        }
                        let ok = (1..=24).contains(&n)
                            && !self.bces[(n - 1) as usize].busy;
                        if ok {
                            let b = &mut self.bces[(n - 1) as usize];
                            if op3 == 2 {
                                b.base = ev;
                            } else {
                                b.pc = ev;
                            }
                        } else {
                            self.err(if op3 == 2 {
                                msc_status::LBB_ERR
                            } else {
                                msc_status::LBP_ERR
                            });
                        }
                    }
                    4 | 5 => {
                        // @LF/@LH (op3=4) and @STF/@STH (op3=5); bit 12 =
                        // T (halfword) (p. II-33..II-36).
                        let half = hw1 & 0x0008 != 0;
                        let mut a = addr18 & 0x3FFFF;
                        if ibit {
                            a = a.wrapping_add(self.msc.x) & 0x3FFFF;
                        }
                        if mbit {
                            a = self.read_f(mem, a) & 0x3FFFF;
                        }
                        if op3 == 4 {
                            self.msc.acc = if half {
                                let h = mem.read_h(a).unwrap_or(0);
                                h as i16 as i32 as u32
                            } else {
                                self.read_f(mem, a)
                            };
                        } else if half {
                            let v = self.msc.acc as u16;
                            let _ = mem.write_h(a, v);
                        } else {
                            let v = self.msc.acc;
                            self.write_f(mem, a, v);
                        }
                    }
                    6 => {
                        // @CI/@C (p. II-47): PC += 2 (less), 3 (equal),
                        // 4 (greater). (yaGPC2 diverges with +1 for
                        // greater; the document's +2/+3/+4 is followed —
                        // see IOP_STATUS.md.)
                        let ev = self.long_ev(mem, addr18, ibit, mbit) as i32;
                        let n = if acc < ev {
                            2
                        } else if acc == ev {
                            3
                        } else {
                            4
                        };
                        self.msc.pc = self.msc.pc.wrapping_add(n) & 0x3FFFF;
                        return;
                    }
                    _ => {
                        // @TMI/@TM (p. II-48): AND with ACC; non-zero ->
                        // skip next halfword.
                        let ev = self.long_ev(mem, addr18, ibit, mbit);
                        let n = if self.msc.acc & ev != 0 { 3 } else { 2 };
                        self.msc.pc = self.msc.pc.wrapping_add(n) & 0x3FFFF;
                        return;
                    }
                }
                self.msc.pc = self.msc.pc.wrapping_add(2) & 0x3FFFF;
                return;
            }
            _ => {
                // Unassigned encoding: illegal-opcode error, MSC stops
                // (App. II §2.4).
                self.err(msc_status::ILLEGAL);
                self.msc.busy = false;
                return;
            }
        }
        self.msc.pc = self.msc.pc.wrapping_add(1) & 0x3FFFF;
    }
}

fn sign18(v: u32) -> i32 {
    if v & 0x20000 != 0 { (v | !0x3FFFF) as i32 } else { v as i32 }
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
                self.mia_rcvr_enable |= data.unwrap_or(0);
                PcResponse::OutputAccepted
            }
            (true, 0x8408_0000) => {
                self.mia_rcvr_enable &= !data.unwrap_or(0);
                PcResponse::OutputAccepted
            }
            (true, 0x8504_0000) => {
                self.mia_xmtr_enable |= data.unwrap_or(0);
                PcResponse::OutputAccepted
            }
            (true, 0x8404_0000) => {
                self.mia_xmtr_enable &= !data.unwrap_or(0);
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
            (false, 0x0818_0000) => PcResponse::Input(self.discrete_in),
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

#[cfg(test)]
mod msc_tests {
    use super::*;
    use crate::mem::Memory;

    fn iop_at(mem: &mut Memory, pc: u32, words: &[u16]) -> Iop {
        mem.load_halfwords(pc, words).unwrap();
        let mut iop = Iop::new();
        iop.halted = false;
        iop.msc.busy = true;
        iop.msc.pc = pc;
        iop
    }

    #[test]
    fn acc_memory_ops() {
        // @L / @A / @ST: PC-relative fullword operands (App. II §3.1).
        let mut mem = Memory::new(0x2000);
        mem.write_f(0x110, 40).unwrap();
        mem.write_f(0x112, 2).unwrap();
        // @L +0x10 ; @A +0x11 ; @ST +0x12  (disp relative to each PC)
        let mut iop = iop_at(
            &mut mem,
            0x100,
            &[0x4010, 0x5011, 0x8012],
        );
        iop.step(&mut mem, &mut LocalBuses::new()); // @L: ACC = mem[0x100+0x10]
        assert_eq!(iop.msc.acc, 40);
        iop.step(&mut mem, &mut LocalBuses::new()); // @A: + mem[0x101+0x11]
        assert_eq!(iop.msc.acc, 42);
        iop.step(&mut mem, &mut LocalBuses::new()); // @ST -> mem[0x102+0x12]
        assert_eq!(mem.read_f(0x114).unwrap(), 42);
        assert_eq!(iop.msc.pc, 0x103);
    }

    #[test]
    fn immediates_and_branch() {
        // @LI -5 ; @TI 10 ; @BC cond>0 back -2 (loops once) — App. II
        // Tables 3.2 and p. II-38.
        let mut mem = Memory::new(0x2000);
        // @LI: 1110 1 111 (opx7) imm8; @TI: 1110 1 101 imm8
        let li = 0b1110_1_111_0000_0000u16 | (-5i8 as u8 as u16);
        let ti = 0b1110_1_101_0000_0000u16 | 10u16;
        // @BC always (cond 111) forward +2
        let bc = 0b0010_0_111_0000_0000u16 | 2;
        let mut iop = iop_at(&mut mem, 0x200, &[li, ti, bc]);
        iop.step(&mut mem, &mut LocalBuses::new());
        assert_eq!(iop.msc.acc as i32, -5);
        iop.step(&mut mem, &mut LocalBuses::new());
        assert_eq!(iop.msc.acc, 5);
        iop.step(&mut mem, &mut LocalBuses::new());
        assert_eq!(iop.msc.pc, 0x203 + 2, "branch adds disp to updated PC");
    }

    #[test]
    fn long_format_bu_call_and_boundary() {
        let mut mem = Memory::new(0x2000);
        // @BU immediate to 0x400: 1111 0 000 00000 0 aa + hw2
        let mut iop = iop_at(&mut mem, 0x300, &[0b1111_0_000_00000_0_00, 0x0400]);
        iop.step(&mut mem, &mut LocalBuses::new());
        assert_eq!(iop.msc.pc, 0x400);
        // @CALL 4,0x500 from 0x400: return addr 0x404 stored at 0x500,
        // branch to 0x502 (App. II p. II-41).
        mem.load_halfwords(0x400, &[0b1111_0_001_00100_0_00, 0x0500]).unwrap();
        iop.step(&mut mem, &mut LocalBuses::new());
        assert_eq!(mem.read_f(0x500).unwrap(), 0x404);
        assert_eq!(iop.msc.pc, 0x502);
        // long instruction at an odd boundary: boundary error, MSC stops
        let mut mem = Memory::new(0x2000);
        let mut iop = iop_at(&mut mem, 0x301, &[0b1111_0_000_00000_0_00, 0x0400]);
        iop.step(&mut mem, &mut LocalBuses::new());
        assert!(!iop.msc.busy);
        assert!(iop.msc_errors & msc_status::BOUNDARY != 0);
    }

    #[test]
    fn tsz_tm_and_ci_sequencing() {
        let mut mem = Memory::new(0x2000);
        mem.write_f(0x150, (-1i32) as u32).unwrap();
        // @TSZ +0x50 at 0x100: -1 + 1 = 0 -> skip next (App. II p. II-46)
        let mut iop = iop_at(&mut mem, 0x100, &[0x9050]);
        iop.step(&mut mem, &mut LocalBuses::new());
        assert_eq!(mem.read_f(0x150).unwrap(), 0);
        assert_eq!(iop.msc.pc, 0x102);
        // @CI immediate: ACC=5 vs 5 -> PC += 3 (p. II-47)
        let mut mem = Memory::new(0x2000);
        let mut iop = iop_at(&mut mem, 0x200, &[0b1111_0_110_00000_0_00, 5]);
        iop.msc.acc = 5;
        iop.step(&mut mem, &mut LocalBuses::new());
        assert_eq!(iop.msc.pc, 0x203);
        // @TM immediate mask: ACC & 6 != 0 -> PC += 3 (p. II-48)
        let mut mem = Memory::new(0x2000);
        let mut iop = iop_at(&mut mem, 0x200, &[0b1111_0_111_00000_0_00, 6]);
        iop.msc.acc = 2;
        iop.step(&mut mem, &mut LocalBuses::new());
        assert_eq!(iop.msc.pc, 0x203);
    }

    #[test]
    fn sio_starts_bces_and_lbb_loads_waiting_only() {
        let mut mem = Memory::new(0x2000);
        // @LBB BCE 1 <- 0x600; @SIO with ACC bit 1; @LBP busy BCE errors
        let lbb = [0b1111_0_010_00001_0_00u16, 0x0600];
        let mut iop = iop_at(&mut mem, 0x100, &lbb);
        iop.step(&mut mem, &mut LocalBuses::new());
        assert_eq!(iop.bces[0].base, 0x600);
        // @SIO: ACC bit 1 (IBM) = BCE1. Give BCE1 a #BU-to-self spin
        // loop at 0 so it stays busy once it starts executing.
        mem.load_halfwords(0, &[0b1111_0_000_00000_0_00, 0x0000]).unwrap();
        mem.load_halfwords(0x102, &[0b1110_0_100_0000_0000]).unwrap();
        iop.msc.acc = 1 << 30;
        iop.step(&mut mem, &mut LocalBuses::new());
        assert!(iop.bces[0].busy);
        // pad to an even boundary (long formats require it), then
        // @LBP to the now-busy BCE: error bits, register untouched
        mem.load_halfwords(0x103, &[0xC000]).unwrap(); // @DLY 0
        mem.load_halfwords(0x104, &[0b1111_0_011_00001_0_00, 0x0700]).unwrap();
        iop.step(&mut mem, &mut LocalBuses::new());
        iop.step(&mut mem, &mut LocalBuses::new());
        assert_eq!(iop.bces[0].pc, 0);
        assert!(iop.msc_errors & msc_status::LBP_ERR != 0);
        assert!(iop.msc_errors & msc_status::PROGRAM_EXCEPTION != 0);
    }

    #[test]
    fn sec_external_call_round_trip() {
        // @SEC with C6 set saves state at (C6) and branches to C6+8;
        // @REC restores it (App. II p. II-43/II-62).
        let mut mem = Memory::new(0x2000);
        // main: @SEC delta 0 at 0x100; called pgm at 0x408: @REC +? ->
        // @REC loads from EA; encode @REC disp so EA = 0x400.
        let sec = 0b1110_0_110_0000_0000u16;
        let mut iop = iop_at(&mut mem, 0x100, &[sec]);
        iop.msc.acc = 0xAABB_CCDD;
        iop.msc.x = 0x155;
        iop.c6 = 0x400;
        iop.step(&mut mem, &mut LocalBuses::new());
        assert_eq!(iop.msc.pc, 0x408, "branched to the external program");
        assert_eq!(iop.c6, 0, "C6 cleared");
        assert_eq!(mem.read_f(0x402).unwrap(), 0xAABB_CCDD, "ACC saved");
        assert_eq!(mem.read_f(0x406).unwrap(), 0x101, "return PC saved");
        // external program returns: @REC with EA back at 0x400
        // short_ea: pc 0x408 + disp -8 -> 0x400
        let rec = 0b1010_0_000_0000_0000u16 | ((-8i16 as u16) & 0x7FF);
        mem.load_halfwords(0x408, &[rec]).unwrap();
        iop.step(&mut mem, &mut LocalBuses::new());
        assert_eq!(iop.msc.acc, 0xAABB_CCDD);
        assert_eq!(iop.msc.x, 0x155);
        assert_eq!(iop.msc.pc, 0x101, "returned to the caller");
    }

    #[test]
    fn wat_and_int() {
        let mut mem = Memory::new(0x2000);
        // @INT level 5 then @WAT
        let mut iop = iop_at(&mut mem, 0x100, &[0b0011_0_000_0000_0101, 0x0800]);
        iop.step(&mut mem, &mut LocalBuses::new());
        assert_eq!(iop.cpu_interrupt, Some(5));
        iop.step(&mut mem, &mut LocalBuses::new());
        assert!(!iop.msc.busy, "@WAT enters the wait state");
    }
}

#[cfg(test)]
mod bce_tests {
    use super::*;
    use crate::mem::Memory;

    fn iop_bce(mem: &mut Memory, n: usize, pc: u32, words: &[u16]) -> Iop {
        mem.load_halfwords(pc, words).unwrap();
        let mut iop = Iop::new();
        iop.halted = false;
        iop.bces[n].busy = true;
        iop.bces[n].pc = pc;
        iop.mia_xmtr_enable = 1 << (31 - n as u32);
        iop.mia_rcvr_enable = 1 << (31 - n as u32);
        iop
    }

    #[test]
    fn bce_registers_and_wait() {
        let mut mem = Memory::new(0x2000);
        // #LBR 0x600 ; #LTOI 5 ; #SIB ; #WAT
        let mut iop = iop_bce(
            &mut mem,
            0,
            0x100,
            &[
                0b1111_0_010_000000_00,
                0x0600,
                0b10110_000_0000_0101,
                0b11101_00000000000,
                0b00001_00000000000,
            ],
        );
        let mut bus = LocalBuses::new();
        iop.step(&mut mem, &mut bus);
        assert_eq!(iop.bces[0].base, 0x600);
        iop.step(&mut mem, &mut bus);
        assert_eq!(iop.bces[0].mto, 5);
        iop.step(&mut mem, &mut bus);
        assert!(iop.bces[0].indicator);
        assert_eq!(iop.indicators, 1 << 30, "BCE 1 = IBM bit 1");
        iop.step(&mut mem, &mut bus);
        assert!(!iop.bces[0].busy, "#WAT enters the wait state");
    }

    #[test]
    fn cmdi_tds_transmit() {
        let mut mem = Memory::new(0x2000);
        // buffer: 3 halfwords at base+4
        mem.load_halfwords(0x604, &[0x1111, 0x2222, 0x3333]).unwrap();
        // #CMDI with IUA 9, low bits 0x21 ; #TDS count 2 (=3 words) disp 4
        let mut iop = iop_bce(
            &mut mem,
            2,
            0x100,
            &[
                0b1111_0_110_01001_000,
                0x0021,
                0b100_00010_00000100,
            ],
        );
        iop.bces[2].base = 0x600;
        let mut bus = LocalBuses::new();
        iop.step(&mut mem, &mut bus); // #CMDI
        assert_eq!(iop.bces[2].iuar, 9, "IUAR from command's top 5 bits");
        assert_eq!(bus.sent.len(), 1);
        assert_eq!(bus.sent[0], (2, BusWord::command((9 << 19) | 0x21)));
        iop.step(&mut mem, &mut bus); // #TDS
        assert_eq!(bus.sent.len(), 4);
        assert_eq!(bus.sent[1], (2, BusWord::data(9, 0x1111)));
        assert_eq!(bus.sent[3], (2, BusWord::data(9, 0x3333)));
        assert!(iop.bces[2].busy, "no error");
        // with the transmitter disabled, #TDS error-terminates (bit 23)
        let mut mem2 = Memory::new(0x2000);
        let mut iop = iop_bce(&mut mem2, 2, 0x100, &[0b100_00010_00000100]);
        iop.mia_xmtr_enable = 0;
        iop.step(&mut mem2, &mut LocalBuses::new());
        assert!(!iop.bces[2].busy);
        assert!(iop.bces[2].status & bce_status::XMT_DISABLED != 0);
        assert!(iop.bces[2].error && iop.bces[2].indicator);
    }

    #[test]
    fn rds_receive_and_errors() {
        // Good stream into base+disp, then timeout and IUA-mismatch cases.
        let mut mem = Memory::new(0x2000);
        let mut iop = iop_bce(&mut mem, 0, 0x100, &[0b011_00001_00001000]); // #RDS c1 d8
        iop.bces[0].base = 0x700;
        iop.bces[0].iuar = 5;
        let mut bus = LocalBuses::new();
        bus.inject(0, BusWord::data(5, 0xAAAA));
        bus.inject(0, BusWord::data(5, 0xBBBB));
        iop.step(&mut mem, &mut bus);
        assert_eq!(mem.read_h(0x708).unwrap(), 0xAAAA);
        assert_eq!(mem.read_h(0x709).unwrap(), 0xBBBB);
        assert!(iop.bces[0].busy);
        // empty bus: initial timeout
        let mut mem = Memory::new(0x2000);
        let mut iop = iop_bce(&mut mem, 0, 0x100, &[0b011_00000_00000000]);
        iop.step(&mut mem, &mut LocalBuses::new());
        assert!(iop.bces[0].status & bce_status::INITIAL_TIMEOUT != 0);
        assert!(!iop.bces[0].busy);
        // wrong IUA: signature mismatch, IUA recorded in IBM bits 8-12
        let mut mem = Memory::new(0x2000);
        let mut iop = iop_bce(&mut mem, 0, 0x100, &[0b011_00000_00000000]);
        iop.bces[0].iuar = 5;
        let mut bus = LocalBuses::new();
        bus.inject(0, BusWord::data(7, 0x1234));
        iop.step(&mut mem, &mut bus);
        assert!(iop.bces[0].status & bce_status::SIG_MISMATCH != 0);
        assert_eq!((iop.bces[0].status >> 19) & 0x1F, 7);
    }

    /// Two GPCs on one shared bus: GPC A commands and transmits, GPC B
    /// sits in listen mode (#WIX), branches through its listen table,
    /// and receives A's data — the App. III §4 mechanism the BFS used to
    /// eavesdrop on PASS, and the seam for the phase-4 redundant set.
    #[test]
    fn two_gpc_listen_mode() {
        struct View<'a> {
            peer: &'a mut std::collections::VecDeque<BusWord>,
            own: &'a mut std::collections::VecDeque<BusWord>,
        }
        impl BusFabric for View<'_> {
            fn transmit(&mut self, _bus: usize, w: BusWord) {
                self.peer.push_back(w);
            }
            fn receive(&mut self, _bus: usize) -> Option<BusWord> {
                self.own.pop_front()
            }
        }
        let mut q_a = VecDeque::new();
        let mut q_b = VecDeque::new();

        // GPC A: #CMDI carrying a Listen Command (common IOP address
        // 01000 = 8, target IUA 8, table index 3), then #TDS of 2 words.
        let mut mem_a = Memory::new(0x2000);
        mem_a.load_halfwords(0x600, &[0xCAFE, 0xF00D]).unwrap();
        let listen = BusWord::listen(8, 3);
        let mut a = iop_bce(
            &mut mem_a,
            0,
            0x100,
            &[
                0b1111_0_110_01000_000u16 | ((listen.info >> 16) & 7) as u16,
                listen.info as u16,
                0b100_00001_00000000,
            ],
        );
        a.bces[0].base = 0x600;

        // GPC B: #WIX with its branch table; entry 3 points at 0x300,
        // where a #RDS receives 2 words into base+0.
        let mut mem_b = Memory::new(0x2000);
        let mut b = iop_bce(&mut mem_b, 0, 0x200, &[0b00100_000_0000_1111]);
        b.mia_xmtr_enable = 0; // transmitter disabled = listen mode (§4.1)
        // table at (0x201 + 0xF + 1) & !1 = 0x210; entry 3 at 0x216
        mem_b.write_f(0x216, 0x300).unwrap();
        mem_b.load_halfwords(0x300, &[0b011_00001_00000000]).unwrap();
        b.bces[0].base = 0x700;

        // A commands; B catches the listen command and branches.
        a.step(&mut mem_a, &mut View { peer: &mut q_b, own: &mut q_a });
        b.step(&mut mem_b, &mut View { peer: &mut q_a, own: &mut q_b });
        assert_eq!(b.bces[0].pc, 0x300, "listen table branch taken");
        assert_eq!(b.bces[0].iuar, 8, "IUAR from the listen command");
        // A transmits data; B receives it.
        a.step(&mut mem_a, &mut View { peer: &mut q_b, own: &mut q_a });
        b.step(&mut mem_b, &mut View { peer: &mut q_a, own: &mut q_b });
        assert_eq!(mem_b.read_h(0x700).unwrap(), 0xCAFE);
        assert_eq!(mem_b.read_h(0x701).unwrap(), 0xF00D);
        assert!(b.bces[0].busy, "clean reception");
        // one GPC's memory never touched the other's
        assert_eq!(mem_a.read_h(0x700).unwrap(), 0);
    }
}
