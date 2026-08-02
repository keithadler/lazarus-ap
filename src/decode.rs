//! Instruction decode for the AP-101S Shuttle instruction set.
//!
//! Encoding scheme (IBM 85-C67-001 §2.2.3-2.2.8, Figures 2-4..2-11, and
//! §13 "AP-101S Op Code Assignments"):
//!
//! - Bits 0-4: 5-bit op code. Bits 5-7: R1 (or M1 mask, or an op-code
//!   extension "OPX" for the immediate groups).
//! - Bits 8-15 select the form:
//!   - bits 8-10 != 111: **SRS** — bits 8-13 displacement, bits 14-15 B2.
//!     "Displacements of the form 111XXX are not valid" (they are the
//!     RR/RS encodings below).
//!   - bits 8-12 = 11100: **RR** — bits 13-15 R2.
//!   - bits 8-12 = 11101: **RR alternate** — a second op-code plane
//!     (§13: "OP12 = 1 causes either RR2 or RS2 operations").
//!   - bits 8-12 = 11110: **RS** — bit 13 AM, bits 14-15 B2, one more
//!     halfword of address specification. For op codes with a dedicated
//!     RS-slot instruction (branches), that instruction; otherwise the
//!     RS (fullword) form of the op code's SRS instruction.
//!   - bits 8-12 = 11111: **RS alternate** (RS2) — a different instruction
//!     sharing the op code (e.g. op 00000: SRS/RR/RS = ADD, RS2 = AST).
//! - Op codes 11110/11111 are the shift instructions: bits 8-13 count,
//!   bits 14-15 shift type (§6.0, Figure 6-1).
//! - Op codes 10100/10110 are the implied/explicit immediate groups with
//!   R1 as op-code extension (§13 "IMPLIED IMMEDIATE"/"EXPLICIT
//!   IMMEDIATE"); 10110 forms carry an immediate-data halfword (RI/SI,
//!   §2.2.6-2.2.7).
//! - Op code 11001 similarly extends via R1 (RS2: 000 STM, 001 SVC,
//!   100 LM, 101 LPS; RR2: 000 SPM) and its SRS region bits 14-15 = 01 is
//!   BVCF. Op code 11011's SRS region is subdivided by bits 14-15:
//!   00 BCF, 10 BCB, 11 BCTB (§5.4/5.6/5.8 encodings).
//! - LFXI (op 10111) and LFLI (op 10001) occupy the whole RR region with
//!   bits 12-15 as a 4-bit immediate-value code (§4.17).
//!
//! Every mnemonic → bit-pattern assignment here was verified against the
//! §13 op-code assignment tables and cross-checked against the Virtual AGC
//! project's yaGPC2 decoder (see docs/SOURCES.md, docs/PRIOR_ART.md).

/// One instruction of the Shuttle instruction set.
///
/// Variants are named after the manual's mnemonics. `NotImplemented` covers
/// instructions whose *encoding* is known (decode succeeds, so we can report
/// exactly what the program tried to run) but whose execution is out of
/// phase-1 scope: floating point, I/O, privileged/status-switching and
/// DSE/stack operations. See docs/ISA_STATUS.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instr {
    // Fixed point (§4)
    A, Ar, Ah, Ahi, Ast,
    C, Cr, Cbl, Ch, Chi, Cist,
    D, Dr,
    Xul, Ial, Ihl,
    L, Lr, La, Lcr, Lfxi, Lh, Lm,
    Msth,
    M, Mr, Mh, Mhi, Mih,
    St, Sth, Stm,
    S, Sr, Sst, Sh,
    Td,
    // Branching (§5)
    Bal, Balr, Bix, Bc, Bcr, Bcb, Bcre, Bcf,
    Bct, Bctr, Bctb, Bvc, Bvcr, Bvcf,
    // Shifts (§6)
    Nct, Sll, Sldl, Sra, Srda, Srl, Srdl, Srr, Srdr,
    // Logical (§7)
    N, Nr, Nhi, Nist, Nst,
    X, Xr, Xhi, Xist, Xst,
    O, Or, Ohi, Ost,
    Sum, Sb, Shw, Tb, Trb, Th, Zb, Zrb, Zh,
    /// Known encoding, execution not implemented in phase 1.
    NotImplemented(&'static str),
}

/// How the second operand / address is specified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operand {
    /// RR: second operand register (or branch-address register).
    R(u8),
    /// SRS (and SI): 6-bit displacement, base register B2 (00-11 = GR0-GR3).
    Srs { d: u8, b2: u8 },
    /// RS extended addressing (AM=0): 16-bit displacement; B2=11 means no
    /// base (displacement used directly). §2.2.8.
    RsExt { d16: u16, b2: u8 },
    /// RS indexed addressing (AM=1): 11-bit displacement, X index register
    /// (0 = no index), IA indirect bit, I bit. §2.2.8.
    RsIdx { d11: u16, b2: u8, x: u8, ia: bool, i: bool },
    /// Shift instructions: 6-bit count field (Figure 6-1).
    Count(u8),
    /// No storage/register operand beyond R1/immediate.
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decoded {
    pub instr: Instr,
    /// Bits 5-7: R1, or M1 for BC/BVC families (never an OPX — those are
    /// consumed during decode).
    pub r1: u8,
    pub operand: Operand,
    /// RI/SI immediate data halfword, or the LFXI/LFLI value code.
    pub imm: u16,
    /// Instruction length in halfwords (1 or 2).
    pub len: u8,
    pub raw: (u16, u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// No instruction is assigned to this encoding in IBM 85-C67-001 §13.
    Illegal { hw1: u16 },
}

/// Second-operand width, used for SRS displacement scaling (§2.2.5,
/// Figures 2-7/2-8) and automatic index alignment (§14.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Width {
    Half,
    Full,
}

impl Instr {
    /// Storage-operand width. LM/STM are fullword-operand but are "excluded
    /// from automatic index alignment and have a halfword index alignment"
    /// (§14.1, §4.19/4.27) — that exception is handled in EA generation,
    /// not here. LA/IAL develop a *halfword* address (§4.12, §4.15).
    /// Branch targets are halfword (instruction) addresses.
    pub fn width(self) -> Width {
        use Instr::*;
        match self {
            A | Ast | C | D | L | M | St | S | Sst
            | N | Nst | X | Xst | O | Ost | Lm | Stm => Width::Full,
            _ => Width::Half,
        }
    }

    /// LM, STM (and LPS/ISPB, unimplemented) always use halfword index
    /// alignment (§14.1).
    pub fn halfword_index_alignment(self) -> bool {
        matches!(self, Instr::Lm | Instr::Stm)
    }

    pub fn mnemonic(self) -> &'static str {
        use Instr::*;
        match self {
            A => "A", Ar => "AR", Ah => "AH", Ahi => "AHI", Ast => "AST",
            C => "C", Cr => "CR", Cbl => "CBL", Ch => "CH", Chi => "CHI",
            Cist => "CIST", D => "D", Dr => "DR", Xul => "XUL", Ial => "IAL",
            Ihl => "IHL", L => "L", Lr => "LR", La => "LA", Lcr => "LCR",
            Lfxi => "LFXI", Lh => "LH", Lm => "LM", Msth => "MSTH", M => "M",
            Mr => "MR", Mh => "MH", Mhi => "MHI", Mih => "MIH", St => "ST",
            Sth => "STH", Stm => "STM", S => "S", Sr => "SR", Sst => "SST",
            Sh => "SH", Td => "TD", Bal => "BAL", Balr => "BALR",
            Bix => "BIX", Bc => "BC", Bcr => "BCR", Bcb => "BCB",
            Bcre => "BCRE", Bcf => "BCF", Bct => "BCT", Bctr => "BCTR",
            Bctb => "BCTB", Bvc => "BVC", Bvcr => "BVCR", Bvcf => "BVCF",
            Nct => "NCT", Sll => "SLL", Sldl => "SLDL", Sra => "SRA",
            Srda => "SRDA", Srl => "SRL", Srdl => "SRDL", Srr => "SRR",
            Srdr => "SRDR", N => "N", Nr => "NR", Nhi => "NHI",
            Nist => "NIST", Nst => "NST", X => "X", Xr => "XR", Xhi => "XHI",
            Xist => "XIST", Xst => "XST", O => "O", Or => "OR", Ohi => "OHI",
            Ost => "OST", Sum => "SUM", Sb => "SB", Shw => "SHW", Tb => "TB",
            Trb => "TRB", Th => "TH", Zb => "ZB", Zrb => "ZRB", Zh => "ZH",
            NotImplemented(m) => m,
        }
    }
}

use Instr::*;

/// The SRS-form instruction for an op code, or `None` if the op code has no
/// SRS region (or it is claimed by a sub-op group handled separately).
fn srs_slot(op5: u8) -> Option<Instr> {
    match op5 {
        0b00000 => Some(A),
        0b00001 => Some(S),
        0b00010 => Some(C),
        0b00011 => Some(L),
        0b00100 => Some(N),
        0b00101 => Some(O),
        0b00110 => Some(St),
        0b00111 => Some(NotImplemented("STE")),
        0b01000 => Some(M),
        0b01001 => Some(D),
        0b01010 => Some(NotImplemented("AE")),
        0b01011 => Some(NotImplemented("SE")),
        0b01100 => Some(NotImplemented("ME")),
        0b01101 => Some(NotImplemented("DE")),
        0b01110 => Some(X),
        0b01111 => Some(NotImplemented("LE")),
        0b10000 => Some(Ah),
        0b10001 => Some(Sh),
        0b10010 => Some(Ch),
        0b10011 => Some(Lh),
        0b10101 => Some(Mh),
        0b10111 => Some(Sth),
        0b11100 => Some(Ial),
        0b11101 => Some(La),
        _ => None,
    }
}

/// RR slot (bits 8-12 = 11100).
fn rr_slot(op5: u8, r2: u8) -> Option<Instr> {
    match op5 {
        0b00000 => Some(Ar),
        0b00001 => Some(Sr),
        0b00010 => Some(Cr),
        0b00011 => Some(Lr),
        0b00100 => Some(Nr),
        0b00101 => Some(Or),
        0b00111 => Some(NotImplemented("CVFX")),
        0b01000 => Some(Mr),
        0b01001 => Some(Dr),
        0b01010 => Some(NotImplemented("AER")),
        0b01011 => Some(NotImplemented("SER")),
        0b01100 => Some(NotImplemented("MER")),
        0b01101 => Some(NotImplemented("DER")),
        0b01110 => Some(Xr),
        0b01111 => Some(NotImplemented("LER")),
        0b11000 => Some(Bcr),
        0b11001 => Some(Bvcr),
        0b11010 => Some(Bctr),
        0b11011 => Some(NotImplemented("ICR")),
        0b11100 => Some(Balr),
        _ => {
            let _ = r2;
            None
        }
    }
}

/// RR-alternate slot (bits 8-12 = 11101).
fn rr2_slot(op5: u8, r1: u8) -> Option<Instr> {
    match op5 {
        0b00000 => Some(Xul),
        0b00001 => Some(Cbl),
        0b00010 => Some(NotImplemented("DEDR")),
        0b00011 => Some(NotImplemented("CEDR")),
        0b00100 => Some(NotImplemented("LFXR")),
        0b00101 => Some(NotImplemented("LFLR")),
        0b00110 => Some(NotImplemented("MEDR")),
        0b00111 => Some(NotImplemented("CVFL")),
        0b01000 => Some(NotImplemented("LXAR")),
        0b01001 => Some(NotImplemented("CER")),
        0b01010 => Some(NotImplemented("AEDR")),
        0b01011 => Some(NotImplemented("SEDR")),
        0b01101 => Some(NotImplemented("MVH")),
        0b01111 => Some(NotImplemented("LECR")),
        0b10010 => Some(NotImplemented("SRET")),
        0b10011 => Some(Sum),
        0b10100 => Some(NotImplemented("STXAR")),
        0b11000 => Some(Bcre),
        0b11001 => match r1 {
            0b000 => Some(NotImplemented("SPM")),
            _ => None,
        },
        0b11011 => Some(NotImplemented("PC")),
        0b11100 => Some(Nct),
        0b11101 => Some(Lcr),
        _ => None,
    }
}

/// Dedicated RS-slot instructions (bits 8-12 = 11110). Op codes not listed
/// here use the RS form of their SRS-slot instruction.
fn rs_slot_dedicated(op5: u8) -> Option<Instr> {
    match op5 {
        0b11000 => Some(Bc),
        0b11001 => Some(Bvc),
        0b11010 => Some(Bct),
        0b11011 => Some(Bix),
        0b11100 => Some(Bal),
        _ => None,
    }
}

/// RS-alternate slot (bits 8-12 = 11111).
fn rs2_slot(op5: u8, r1: u8) -> Option<Instr> {
    match op5 {
        0b00000 => Some(Ast),
        0b00001 => Some(Sst),
        0b00010 => Some(NotImplemented("DED")),
        0b00011 => Some(NotImplemented("CED")),
        0b00100 => Some(Nst),
        0b00101 => Some(Ost),
        0b00110 => Some(NotImplemented("MED")),
        0b00111 => Some(NotImplemented("STED")),
        0b01000 => Some(NotImplemented("LXA")),
        0b01001 => Some(NotImplemented("CE")),
        0b01010 => Some(NotImplemented("AED")),
        0b01011 => Some(NotImplemented("SED")),
        0b01100 => Some(NotImplemented("MVS")),
        0b01101 => match r1 {
            0b000 => Some(NotImplemented("LDM")),
            _ => None,
        },
        0b01110 => Some(Xst),
        0b01111 => Some(NotImplemented("LED")),
        0b10000 => Some(Ihl),
        0b10001 => match r1 {
            0b000 => Some(NotImplemented("SSM")),
            _ => None,
        },
        0b10010 => match r1 {
            0b000 => Some(NotImplemented("STDM")),
            _ => None,
        },
        0b10011 => Some(Mih),
        0b10100 => Some(NotImplemented("STXA")),
        0b10111 => match r1 {
            0b000 => Some(NotImplemented("TS")),
            _ => None,
        },
        0b11000 => Some(NotImplemented("DIAG")),
        0b11001 => match r1 {
            0b000 => Some(Stm),
            0b001 => Some(NotImplemented("SVC")),
            0b100 => Some(Lm),
            0b101 => Some(NotImplemented("LPS")),
            _ => None,
        },
        0b11010 => Some(NotImplemented("SCAL")),
        // IAL's RS forms live in the RS-alternate slot because BAL owns the
        // RS slot of op code 11100 (§13; yaGPC2 handles IAL identically).
        0b11100 => Some(Ial),
        0b11101 => Some(NotImplemented("ISPB")),
        _ => None,
    }
}

/// Implied-immediate group, op code 10100, R1 = OPX (§13).
fn implied_imm(opx: u8) -> Option<Instr> {
    match opx {
        0b000 => Some(Td),
        0b001 => Some(Zh),
        0b010 => Some(Shw),
        0b011 => Some(Th),
        _ => None,
    }
}

/// Explicit-immediate group, op code 10110, R1 = OPX (§13). `.0` is the
/// RR(-with-immediate, i.e. RI) instruction, `.1` the SRS-with-immediate
/// (SI) instruction.
fn explicit_imm(opx: u8) -> (Option<Instr>, Option<Instr>) {
    match opx {
        0b000 => (Some(Ahi), Some(Msth)),
        0b001 => (Some(Zrb), Some(Zb)),
        0b010 => (Some(Ohi), Some(Sb)),
        0b011 => (Some(Trb), Some(Tb)),
        0b100 => (Some(Xhi), Some(Xist)),
        0b101 => (Some(Chi), Some(Cist)),
        0b110 => (Some(Nhi), Some(Nist)),
        0b111 => (Some(Mhi), Some(NotImplemented("TSB"))),
        _ => (None, None),
    }
}

pub fn decode(hw1: u16, hw2: u16) -> Result<Decoded, DecodeError> {
    let op5 = ((hw1 >> 11) & 0x1F) as u8;
    let r1 = ((hw1 >> 8) & 7) as u8;
    let low = (hw1 & 0xFF) as u8;
    let illegal = Err(DecodeError::Illegal { hw1 });

    let mk = |instr, r1, operand, imm, len| {
        Ok(Decoded { instr, r1, operand, imm, len, raw: (hw1, hw2) })
    };

    // Shift instructions: op codes 11110 (single) / 11111 (double);
    // bits 8-13 count field, bits 14-15 type (§6.0).
    if op5 == 0b11110 || op5 == 0b11111 {
        let dbl = op5 & 1 == 1;
        let count = (low >> 2) & 0x3F;
        let instr = match (dbl, low & 0b11) {
            (false, 0b00) => Sll,
            (false, 0b01) => Sra,
            (false, 0b10) => Srl,
            (false, 0b11) => Srr,
            (true, 0b00) => Sldl,
            (true, 0b01) => Srda,
            (true, 0b10) => Srdl,
            (true, 0b11) => Srdr,
            _ => unreachable!(),
        };
        return mk(instr, r1, Operand::Count(count), 0, 1);
    }

    // SRS region: bits 8-10 != 111.
    if low & 0b1110_0000 != 0b1110_0000 {
        let d = (low >> 2) & 0x3F;
        let b2 = low & 0b11;
        return match op5 {
            // SRS-format relative branches use bits 14-15 as a sub-op.
            0b11001 => match b2 {
                0b01 => mk(Bvcf, r1, Operand::Srs { d, b2: 0 }, 0, 1),
                _ => illegal,
            },
            0b11011 => match b2 {
                0b00 => mk(Bcf, r1, Operand::Srs { d, b2: 0 }, 0, 1),
                0b10 => mk(Bcb, r1, Operand::Srs { d, b2: 0 }, 0, 1),
                0b11 => mk(Bctb, r1, Operand::Srs { d, b2: 0 }, 0, 1),
                _ => illegal,
            },
            // Implied-immediate group: R1 is an op-code extension.
            0b10100 => match implied_imm(r1) {
                Some(i) => mk(i, 0, Operand::Srs { d, b2 }, 0, 1),
                None => illegal,
            },
            // Explicit-immediate group, SI form: immediate halfword follows.
            0b10110 => match explicit_imm(r1).1 {
                Some(i) => mk(i, 0, Operand::Srs { d, b2 }, hw2, 2),
                None => illegal,
            },
            _ => match srs_slot(op5) {
                Some(i) => mk(i, r1, Operand::Srs { d, b2 }, 0, 1),
                None => illegal,
            },
        };
    }

    // RR region: bits 8-11 = 1110.
    if low & 0b0001_0000 == 0 {
        // LFXI / LFLI occupy the whole RR region of their op codes, using
        // bits 12-15 as the value code (§4.17).
        if op5 == 0b10111 {
            return mk(Lfxi, r1, Operand::None, (low & 0xF) as u16, 1);
        }
        if op5 == 0b10001 {
            return mk(NotImplemented("LFLI"), r1, Operand::None, (low & 0xF) as u16, 1);
        }
        let r2 = low & 0b111;
        if low & 0b0000_1000 == 0 {
            // Primary RR slot (bits 8-12 = 11100).
            if op5 == 0b10110 {
                // Explicit-immediate group, RI form: R1 = OPX, register in
                // bits 13-15, immediate halfword follows (§2.2.7).
                return match explicit_imm(r1).0 {
                    Some(i) => mk(i, r2, Operand::None, hw2, 2),
                    None => illegal,
                };
            }
            match rr_slot(op5, r2) {
                Some(i) => mk(i, r1, Operand::R(r2), 0, 1),
                None => illegal,
            }
        } else {
            // Alternate RR slot (bits 8-12 = 11101).
            match rr2_slot(op5, r1) {
                Some(i) => mk(i, r1, Operand::R(r2), 0, 1),
                None => illegal,
            }
        }
    } else {
        // RS region: bits 8-11 = 1111; bit 12 selects RS vs RS-alternate;
        // bit 13 is AM; bits 14-15 B2; second halfword follows (§2.2.8).
        let am = low & 0b0000_0100 != 0;
        let b2 = low & 0b11;
        let operand = if !am {
            Operand::RsExt { d16: hw2, b2 }
        } else {
            Operand::RsIdx {
                d11: hw2 & 0x7FF,
                b2,
                x: ((hw2 >> 13) & 7) as u8,
                ia: hw2 & (1 << 12) != 0,
                i: hw2 & (1 << 11) != 0,
            }
        };
        // `opx_consumed`: the R1 field acted as an op-code extension and
        // must not reach execution as a register/mask number.
        let (slot, opx_consumed) = if low & 0b0000_1000 == 0 {
            // RS slot: dedicated instruction, or RS form of the SRS
            // instruction (including the implied-immediate group; the SI
            // group has no RS forms).
            match rs_slot_dedicated(op5) {
                Some(i) => (Some(i), false),
                None => match op5 {
                    0b10100 => (implied_imm(r1), true),
                    0b10110 => (None, false),
                    _ => (srs_slot(op5), false),
                },
            }
        } else {
            let grouped = matches!(op5, 0b01101 | 0b10001 | 0b10010 | 0b10111 | 0b11001);
            (rs2_slot(op5, r1), grouped)
        };
        let eff_r1 = if opx_consumed { 0 } else { r1 };
        match slot {
            Some(i) => mk(i, eff_r1, operand, 0, 2),
            None => illegal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d1(hw1: u16) -> Decoded {
        decode(hw1, 0).unwrap()
    }

    // Encodings below are the §13 op-code assignments; each test constructs
    // the bit pattern from the manual's field layout.

    #[test]
    fn rr_forms() {
        // AR R1,R2: op 00000, bits 8-12 = 11100. AR 3,5 = 0000 0011 1110 0101
        let d = d1(0b00000_011_11100_101);
        assert_eq!(d.instr, Instr::Ar);
        assert_eq!(d.r1, 3);
        assert_eq!(d.operand, Operand::R(5));
        assert_eq!(d.len, 1);
        // SR
        assert_eq!(d1(0b00001_001_11100_010).instr, Instr::Sr);
        // XUL is the RR-alternate of op 00000
        assert_eq!(d1(0b00000_011_11101_101).instr, Instr::Xul);
        // LCR is the RR-alternate of op 11101
        assert_eq!(d1(0b11101_010_11101_001).instr, Instr::Lcr);
        // BALR
        assert_eq!(d1(0b11100_110_11100_001).instr, Instr::Balr);
    }

    #[test]
    fn srs_forms() {
        // A R1,disp(B2): op 00000. A 2,10(1)
        let d = d1(0b00000_010_001010_01);
        assert_eq!(d.instr, Instr::A);
        assert_eq!(d.r1, 2);
        assert_eq!(d.operand, Operand::Srs { d: 10, b2: 1 });
        assert_eq!(d.len, 1);
        // LH 1,55(3): maximum valid displacement 110111
        let d = d1(0b10011_001_110111_11);
        assert_eq!(d.instr, Instr::Lh);
        assert_eq!(d.operand, Operand::Srs { d: 55, b2: 3 });
    }

    #[test]
    fn rs_forms() {
        // A 1,d16(2) extended: op 00000, low byte 11110|0|10, hw2 = disp
        let d = decode(0b00000_001_11110_010, 0x1234).unwrap();
        assert_eq!(d.instr, Instr::A);
        assert_eq!(d.operand, Operand::RsExt { d16: 0x1234, b2: 2 });
        assert_eq!(d.len, 2);
        // AST is the RS-alternate of op 00000
        let d = decode(0b00000_001_11111_010, 0x0010).unwrap();
        assert_eq!(d.instr, Instr::Ast);
        // Indexed: AM=1, hw2 = X|IA|I|d11
        let hw2 = (0b011u16 << 13) | (1 << 12) | (0 << 11) | 0x155;
        let d = decode(0b00000_001_11110_110, hw2).unwrap();
        assert_eq!(
            d.operand,
            Operand::RsIdx { d11: 0x155, b2: 2, x: 3, ia: true, i: false }
        );
        // BAL owns the RS slot of op 11100; IAL's RS form is in RS-alt.
        assert_eq!(decode(0b11100_001_11110_000, 0).unwrap().instr, Instr::Bal);
        assert_eq!(decode(0b11100_001_11111_000, 0).unwrap().instr, Instr::Ial);
    }

    #[test]
    fn immediate_groups() {
        // AHI R2,data: op 10110, OPX=000, bits 8-12 = 11100, R2 in 13-15.
        let d = decode(0b10110_000_11100_100, 0x00FF).unwrap();
        assert_eq!(d.instr, Instr::Ahi);
        assert_eq!(d.r1, 4); // register operand
        assert_eq!(d.imm, 0x00FF);
        assert_eq!(d.len, 2);
        // CHI is OPX=101
        assert_eq!(decode(0b10110_101_11100_001, 0).unwrap().instr, Instr::Chi);
        // MSTH: SI form of OPX=000
        let d = decode(0b10110_000_000100_01, 0xFFFF).unwrap();
        assert_eq!(d.instr, Instr::Msth);
        assert_eq!(d.operand, Operand::Srs { d: 4, b2: 1 });
        assert_eq!(d.imm, 0xFFFF);
        // TD: implied-immediate group, OPX=000, SRS form, len 1
        let d = d1(0b10100_000_000011_10);
        assert_eq!(d.instr, Instr::Td);
        assert_eq!(d.len, 1);
        // LFXI 5, value code 7 (= immediate 5): op 10111, bits 8-11=1110
        let d = d1(0b10111_101_1110_0111);
        assert_eq!(d.instr, Instr::Lfxi);
        assert_eq!(d.r1, 5);
        assert_eq!(d.imm, 7);
    }

    #[test]
    fn branches() {
        // BC M1,...: op 11000 RS slot
        let d = decode(0b11000_111_11110_011, 0x0100).unwrap();
        assert_eq!(d.instr, Instr::Bc);
        assert_eq!(d.r1, 0b111);
        // BCR / BCRE
        assert_eq!(d1(0b11000_001_11100_010).instr, Instr::Bcr);
        assert_eq!(d1(0b11000_001_11101_010).instr, Instr::Bcre);
        // BCF/BCB/BCTB: op 11011 SRS region, bits 14-15 sub-op
        assert_eq!(d1(0b11011_111_000100_00).instr, Instr::Bcf);
        assert_eq!(d1(0b11011_111_000100_10).instr, Instr::Bcb);
        assert_eq!(d1(0b11011_001_000100_11).instr, Instr::Bctb);
        // BVCF: op 11001 SRS region, bits 14-15 = 01
        assert_eq!(d1(0b11001_011_000100_01).instr, Instr::Bvcf);
        // BIX: RS slot of op 11011
        assert_eq!(decode(0b11011_001_11110_011, 0).unwrap().instr, Instr::Bix);
    }

    #[test]
    fn shifts() {
        // SLL R1,count: op 11110, type 00
        let d = d1(0b11110_010_001111_00);
        assert_eq!(d.instr, Instr::Sll);
        assert_eq!(d.operand, Operand::Count(15));
        assert_eq!(d1(0b11110_010_000001_01).instr, Instr::Sra);
        assert_eq!(d1(0b11110_010_000001_10).instr, Instr::Srl);
        assert_eq!(d1(0b11110_010_000001_11).instr, Instr::Srr);
        assert_eq!(d1(0b11111_010_000001_00).instr, Instr::Sldl);
        assert_eq!(d1(0b11111_010_000001_01).instr, Instr::Srda);
        assert_eq!(d1(0b11111_010_000001_10).instr, Instr::Srdl);
        assert_eq!(d1(0b11111_010_000001_11).instr, Instr::Srdr);
        // NCT is the RR-alternate of op 11100
        assert_eq!(d1(0b11100_001_11101_010).instr, Instr::Nct);
    }

    #[test]
    fn register_group_ops() {
        // STM/LM: op 11001 RS-alternate, R1 = OPX
        assert_eq!(decode(0b11001_000_11111_000, 0).unwrap().instr, Instr::Stm);
        assert_eq!(decode(0b11001_100_11111_000, 0).unwrap().instr, Instr::Lm);
        // SUM: RR-alternate of op 10011
        assert_eq!(d1(0b10011_001_11101_010).instr, Instr::Sum);
        // IHL: RS-alternate of op 10000
        assert_eq!(decode(0b10000_001_11111_000, 0).unwrap().instr, Instr::Ihl);
        // MIH: RS-alternate of op 10011
        assert_eq!(decode(0b10011_001_11111_000, 0).unwrap().instr, Instr::Mih);
    }

    #[test]
    fn illegal_encodings() {
        // Op 11000 has no SRS region (§13).
        assert!(decode(0b11000_001_000100_01, 0).is_err());
        // Op 11001 SRS region: only bits 14-15 = 01 (BVCF) is assigned.
        assert!(decode(0b11001_001_000100_00, 0).is_err());
        // ST has no RR form.
        assert!(decode(0b00110_001_11100_010, 0).is_err());
    }

    #[test]
    fn not_implemented_decodes() {
        // Floating point decodes (so traps can name it) but is Unimplemented.
        assert_eq!(
            d1(0b01010_001_11100_010).instr,
            Instr::NotImplemented("AER")
        );
        assert_eq!(
            decode(0b11001_101_11111_000, 0).unwrap().instr,
            Instr::NotImplemented("LPS")
        );
    }
}
