// Copyright (c) 2026 Lazarus AP contributors
//
// Ported from yaShuttle/yaGPC2/src/timing.c in the Virtual AGC project,
// which is licensed under the GNU General Public License version 2 or
// later. This file, and this project as a whole, are licensed on the
// same terms. See LICENSE and NOTICE.
//
// The underlying numbers are not Virtual AGC's: they come from the
// Space Shuttle HAL/S-FC compiler's own EXECUTION_TIMES procedure in
// PASS2.PROCS/OBJECTGE.xpl, which is public domain. What is inherited
// here is the reconstruction of the compiler's private instruction
// index into mnemonics, and the shape of the resolver.

//! Per-instruction AP-101S execution times.
//!
//! # Provenance
//!
//! These numbers are not ours and are not estimates. They come from the
//! real HAL/S-FC PASS2 compiler's own `EXECUTION_TIMES` procedure,
//! nested in `GENERATE_OPERANDS` in
//! `PASS.REL32V0/PASS2.PROCS/OBJECTGE.xpl` — the same computation that
//! printed the `TIME: X.XX` annotations in a Shuttle compile listing.
//! The compiler carries a 95-entry `TIMES()` string table plus three
//! parallel per-instruction index arrays (`NORMAL_TIMES`,
//! `INDIRECT_TIMES`, `INDEX_TIMES`), selected by addressing mode.
//!
//! The historical arrays are keyed by a compiler-private `INST` row
//! number (0-205), not by hardware opcode. Reconstructing that mapping
//! to mnemonics was done by the Virtual AGC project in
//! `yaShuttle/yaGPC2/src/timing.c`, cross-referencing `OBJECTGE.xpl`
//! against `##DRIVER.xpl`'s `AP101INST`/`OPNAMES`/`OPER` tables. This
//! module ports that mnemonic-keyed form. Both sources are cited in
//! docs/SOURCES.md.
//!
//! # UNVERIFIED: the unit
//!
//! HAL/S-FC prints these as bare numbers with no unit anywhere in the
//! listing or the compiler source. Microseconds is the conventional
//! assumption for this hardware generation, and it makes the resulting
//! frame budgets come out plausible, but **this project has not
//! confirmed it against IBM 85-C67-001 §17**. Everything here is
//! therefore reported in "time units" and only *called* microseconds
//! where a caller opts in. Do not present a derived millisecond figure
//! as established fact.
//!
//! # Known gaps
//!
//! - Mnemonics HAL/S-FC never emitted have no entry and return `None`
//!   (e.g. `SRDR`, and the BFS-only branches `BVC`/`BVCF`).
//! - `ME`'s historical special case also fired on a compile-time
//!   condition (`LHS=SRSTYPE`) about the destination's source-level
//!   storage class. That is not encoded in the object code and cannot
//!   be recovered at runtime, so only the odd-register half is applied.
//! - `MVH` was parametric and the compiler could resolve it only when a
//!   preceding `IAL` supplied the count. An emulator always knows the
//!   real count, so it is resolved unconditionally.

use crate::decode::{Decoded, Operand};

/// `TIMES()` indices 0-83: plain values, verbatim from OBJECTGE.xpl.
const PLAIN: [f64; 84] = [
    0.0, 0.25, 0.5, 0.75, 1.0, 1.2, 1.35, 1.5, 1.7, 1.75, 2.0, 2.15, 2.25, 2.4, 2.5, 3.0, 3.25,
    3.75, 4.0, 4.25, 4.5, 4.675, 4.75, 4.925, 5.0, 5.23, 5.25, 5.5, 5.58, 5.75, 6.0, 6.03, 6.25,
    6.28, 6.5, 6.75, 7.0, 7.25, 7.5, 7.55, 7.75, 7.98, 8.0, 8.025, 8.25, 8.5, 8.75, 8.8, 9.0, 9.5,
    9.75, 10.0, 10.05, 10.25, 10.28, 10.5, 10.53, 11.5, 11.75, 11.8, 12.0, 12.5, 12.75, 13.25,
    13.5, 14.25, 15.25, 16.25, 17.5, 18.125, 18.5, 19.0, 20.25, 22.25, 22.5, 22.75, 23.0, 24.0,
    24.5, 25.0, 25.75, 26.75, 27.75, 29.75,
];

/// Indices 85-90: `BT=x, BNT=y` branch-taken / not-taken pairs.
const BRANCH_TAKEN: [f64; 6] = [5.75, 3.50, 1.75, 1.25, 2.5, 1.25];
const BRANCH_NOT_TAKEN: [f64; 6] = [0.50, 4.50, 0.750, 0.50, 1.5, 0.250];

/// Indices 91-94: `base + perUnit * N` shift-count formulas.
const SHIFT_BASE: [f64; 4] = [0.650, 1.0, 0.675, 1.0];
const SHIFT_PER: [f64; 4] = [0.1, 0.25, 0.1, 0.1];

/// (mnemonic, NORMAL_TIMES, INDIRECT_TIMES, INDEX_TIMES). A zero in the
/// indirect/index columns means the mnemonic has no indexed form, which
/// matches the historical all-zero entries.
const TABLE: &[(&str, u8, u8, u8)] = &[
    ("LFXI", 3, 0, 0), ("LFLI", 3, 0, 0), ("SPM", 26, 0, 0), ("BALR", 86, 0, 0),
    ("BCTR", 87, 0, 0), ("BCR", 1, 0, 0), ("SRET", 68, 0, 0), ("MVH", 84, 0, 0),
    ("BCRE", 85, 0, 0), ("LCR", 2, 0, 0), ("NR", 1, 0, 0), ("OR", 1, 0, 0),
    ("XR", 1, 0, 0), ("LR", 1, 0, 0), ("CR", 1, 0, 0), ("AR", 1, 0, 0),
    ("SR", 1, 0, 0), ("MR", 13, 0, 0), ("DR", 23, 0, 0), ("CVFX", 12, 0, 0),
    ("SEDR", 32, 0, 0), ("CEDR", 7, 0, 0), ("AEDR", 32, 0, 0), ("MEDR", 70, 0, 0),
    ("DEDR", 75, 0, 0), ("LECR", 4, 0, 0), ("SER", 12, 0, 0), ("LER", 4, 0, 0),
    ("CER", 7, 0, 0), ("AER", 12, 0, 0), ("MER", 30, 0, 0), ("DER", 37, 0, 0),
    ("CVFL", 9, 0, 0),
    ("STH", 2, 19, 45), ("LA", 1, 17, 42), ("IHL", 2, 20, 37), ("BIX", 89, 27, 44),
    ("BAL", 17, 51, 49), ("BCT", 87, 19, 36), ("BC", 90, 18, 32), ("LH", 1, 19, 36),
    ("CH", 1, 19, 36), ("AH", 1, 19, 36), ("SH", 1, 19, 37), ("MH", 6, 25, 41),
    ("SCAL", 69, 78, 77), ("MIH", 8, 28, 43), ("IAL", 2, 17, 42), ("ST", 2, 20, 48),
    ("N", 1, 20, 34), ("O", 1, 20, 34), ("X", 1, 20, 38), ("L", 1, 19, 37),
    ("C", 1, 19, 37), ("A", 1, 19, 37), ("S", 1, 19, 37), ("M", 13, 33, 56),
    ("D", 23, 47, 59),
    ("STED", 4, 24, 38), ("LED", 7, 24, 46), ("CED", 29, 49, 61), ("AED", 34, 53, 63),
    ("SED", 34, 55, 64), ("MED", 71, 73, 80), ("DED", 76, 82, 83),
    ("STE", 2, 20, 38), ("LE", 5, 22, 45), ("CE", 9, 29, 45), ("AE", 14, 34, 48),
    ("SE", 14, 20, 49), ("ME", 32, 53, 63), ("DE", 38, 57, 66), ("MVS", 22, 48, 58),
    ("BCTB", 87, 0, 0), ("BCF", 1, 0, 0), ("SRL", 91, 0, 0), ("SLL", 93, 0, 0),
    ("SRA", 91, 0, 0), ("SRDL", 94, 0, 0), ("SLDL", 92, 0, 0), ("SRDA", 92, 0, 0),
    ("STM", 37, 51, 65), ("TH", 9, 24, 40), ("TS", 17, 32, 48), ("SHW", 7, 19, 45),
    ("LM", 45, 60, 67), ("SVC", 72, 74, 81), ("TD", 15, 27, 44), ("ZH", 7, 19, 45),
    ("TRB", 4, 0, 0), ("NHI", 1, 0, 0), ("OHI", 1, 0, 0), ("XHI", 1, 0, 0),
    ("LHI", 1, 0, 0), ("CHI", 1, 0, 0), ("AHI", 1, 0, 0), ("MHI", 6, 0, 0),
    ("ZRB", 1, 0, 0), ("TB", 10, 0, 0), ("TSB", 15, 0, 0), ("NIST", 15, 0, 0),
    ("SB", 15, 0, 0), ("XIST", 15, 0, 0), ("CIST", 7, 0, 0), ("MSTH", 15, 0, 0),
    ("ZB", 16, 0, 0),
    ("NST", 3, 29, 53), ("OST", 3, 29, 53), ("XST", 3, 29, 53), ("AST", 3, 29, 53),
    ("SST", 4, 0, 0), ("LDM", 35, 51, 53), ("SRR", 91, 0, 0),
];

/// The operand value a timing lookup needs from register state that is
/// not safe to re-derive afterwards. Capture this *before* executing.
///
/// `MVH` overwrites its own count register, and shift counts come from
/// the instruction's count field. Everything else returns 0.
pub fn pre_n(dec: &Decoded, r1_value: u32) -> u32 {
    match dec.operand {
        Operand::Count(c) => c as u32,
        _ if dec.instr.mnemonic() == "MVH" => r1_value & 0xFFFF,
        _ => 0,
    }
}

/// Whether the instruction uses an index register (`X` non-zero).
fn indexed(dec: &Decoded) -> bool {
    matches!(dec.operand, Operand::RsIdx { x, .. } if x != 0)
}

/// The historical `(SHL(F,1) + IA) > 0` test: either extended-indirect
/// bit set. `F` is the I bit, `IA` the indirect-address bit.
fn extended_indirect(dec: &Decoded) -> bool {
    matches!(dec.operand, Operand::RsIdx { ia, i, .. } if ia || i)
}

fn resolve(idx: u8, pre_n: u32, branch_taken: bool) -> f64 {
    let idx = idx as usize;
    if idx == 0 {
        return 0.0; // historical unused slot: no time recorded
    }
    if idx < 84 {
        return PLAIN[idx];
    }
    if idx == 84 {
        // MVH, the one genuinely parametric case. The compiler could
        // only resolve this when a preceding IAL gave it the count; we
        // always know the real count, so the resolved rules apply
        // unconditionally.
        let n = (pre_n & 0xFFFF) as i16;
        return match n {
            1 => 11.25,
            0 => 7.75,
            n if n < 0 => 7.5,
            n if n % 2 == 0 => 10.25 + 0.875 * n as f64,
            n => 12.0 + 0.875 * (n as f64 - 1.0),
        };
    }
    if idx <= 90 {
        return if branch_taken {
            BRANCH_TAKEN[idx - 85]
        } else {
            BRANCH_NOT_TAKEN[idx - 85]
        };
    }
    if idx <= 94 {
        return SHIFT_BASE[idx - 91] + SHIFT_PER[idx - 91] * pre_n as f64;
    }
    0.0
}

/// Execution time for one instruction, in HAL/S-FC's own units (see the
/// module header: the unit itself is UNVERIFIED).
///
/// `pre_n` comes from [`pre_n`], captured before execution.
/// `branch_taken` matters only for the branch mnemonics whose time
/// genuinely depends on the outcome; pass it regardless.
///
/// Returns `None` for any mnemonic HAL/S-FC never timed, so callers can
/// tell "no time recorded" apart from "zero time".
pub fn instr_time(dec: &Decoded, pre_n: u32, branch_taken: bool) -> Option<f64> {
    let nm = dec.instr.mnemonic();
    let odd_r = dec.r1 & 1 != 0;
    let ix = indexed(dec);
    let poo = extended_indirect(dec);

    // An odd destination register makes the multiply/divide family take
    // a fixed, higher time (the register-pair complication). Even R
    // falls through to the ordinary lookup.
    if odd_r {
        match nm {
            "MR" => return Some(PLAIN[11]),
            "DR" => return Some(PLAIN[21]),
            "MER" => return Some(PLAIN[27]),
            "M" => {
                return Some(if ix {
                    if poo { PLAIN[31] } else { PLAIN[54] }
                } else {
                    PLAIN[21]
                })
            }
            "D" => {
                return Some(if ix {
                    if poo { PLAIN[39] } else { PLAIN[52] }
                } else {
                    PLAIN[21]
                })
            }
            "ME" => {
                return Some(if ix {
                    if poo { PLAIN[50] } else { PLAIN[62] }
                } else {
                    PLAIN[29]
                })
            }
            _ => {}
        }
    }

    let &(_, normal, indirect, index) = TABLE.iter().find(|e| e.0 == nm)?;
    let idx = if (indirect != 0 || index != 0) && ix {
        if poo { indirect } else { index }
    } else {
        normal
    };
    Some(resolve(idx, pre_n, branch_taken))
}

/// A running total of execution time over a program run.
#[derive(Debug, Default, Clone, Copy)]
pub struct Budget {
    /// Total time in HAL/S-FC units.
    pub units: f64,
    /// Instructions that had a timing entry.
    pub timed: u64,
    /// Instructions HAL/S-FC never timed (excluded from `units`).
    pub untimed: u64,
}

impl Budget {
    pub fn add(&mut self, t: Option<f64>) {
        match t {
            Some(v) => {
                self.units += v;
                self.timed += 1;
            }
            None => self.untimed += 1,
        }
    }

    /// Fraction of a 40 Hz frame (25 000 units) this run would occupy,
    /// **on the unconfirmed assumption that a unit is one microsecond**.
    /// See the module header before quoting this anywhere.
    pub fn frames_at_40hz(&self) -> f64 {
        self.units / 25_000.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::decode;

    fn t(hw1: u16, hw2: u16) -> Option<f64> {
        let d = decode(hw1, hw2).unwrap();
        instr_time(&d, pre_n(&d, 0), false)
    }

    #[test]
    fn register_ops_take_their_tabled_time() {
        // LR is a plain register load: NORMAL_TIMES index 1 -> 0.25.
        let d = decode(0x18E0, 0).unwrap();
        assert_eq!(d.instr.mnemonic(), "LR");
        assert_eq!(instr_time(&d, 0, false), Some(0.25));
    }

    #[test]
    fn an_odd_destination_register_costs_more() {
        // MR with an even R1 uses index 13 (2.4); an odd R1 uses the
        // fixed index 11 (2.15) -- both from OBJECTGE.xpl.
        let even = decode(0x40E0, 0).unwrap();
        let odd = decode(0x41E0, 0).unwrap();
        assert_eq!(even.instr.mnemonic(), "MR");
        assert_eq!(odd.instr.mnemonic(), "MR");
        assert_eq!(instr_time(&even, 0, false), Some(PLAIN[13]));
        assert_eq!(instr_time(&odd, 0, false), Some(PLAIN[11]));
    }

    #[test]
    fn mvh_time_scales_with_its_count() {
        let d = decode(0x68E8, 0).unwrap();
        assert_eq!(d.instr.mnemonic(), "MVH");
        // The resolved rules: 0 -> 7.75, 1 -> 11.25, negative -> 7.5,
        // even n -> 10.25 + 0.875n, odd n -> 12.0 + 0.875(n-1).
        assert_eq!(instr_time(&d, 0, false), Some(7.75));
        assert_eq!(instr_time(&d, 1, false), Some(11.25));
        assert_eq!(instr_time(&d, 4, false), Some(10.25 + 0.875 * 4.0));
        assert_eq!(instr_time(&d, 5, false), Some(12.0 + 0.875 * 4.0));
        assert_eq!(instr_time(&d, 0xFFFF, false), Some(7.5)); // -1
    }

    #[test]
    fn a_budget_accumulates_and_reports_frames() {
        let mut b = Budget::default();
        b.add(Some(10.0));
        b.add(None);
        b.add(Some(15.0));
        assert_eq!(b.units, 25.0);
        assert_eq!((b.timed, b.untimed), (2, 1));
        assert!((b.frames_at_40hz() - 0.001).abs() < 1e-9);
        let _ = t(0x18E0, 0);
    }
}
