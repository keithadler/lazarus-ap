//! A tiny AP-101S assembler, sufficient to write readable test programs.
//!
//! This is a test harness, not a reconstruction of the historical AP-101
//! assembler; the operand syntax follows the instruction pages of IBM
//! 85-C67-001 (e.g. `A R1,D2(B2)`, `A R1,D2(X2,B2)`, `AHI R2,data`), and
//! the mnemonics are the manual's. Supported statements:
//!
//! ```text
//! ; comment (also '*' in column 1)
//! LABEL:  AR   3,5          ; RR
//!         A    2,10(1)      ; SRS if the displacement fits, else RS
//!         A    2,LABEL      ; RS extended, B2=11 (address = displacement)
//!         A    2,10(3,1)    ; RS indexed: D2(X2,B2)
//!         A@   2,10(0,1)    ; indirect (IA=1)
//!         A#   2,10(3,1)    ; automatic index modification (I=1)
//!         AHI  2,-100       ; RI immediate
//!         MSTH 4(1),1       ; SI: D2(B2),data
//!         LFXI 3,7          ; immediate value -2..13
//!         SLL  1,5          ; shift, count 0-55 (56-63 = computed)
//!         BC   7,LABEL      ; branch (RS extended, B2=11)
//!         B    LABEL        ; alias for BC 7,...
//!         NOP               ; alias for BCR 0,0
//!         BCF  7,LABEL      ; short relative branches; the assembler
//!         BCB  7,LABEL      ;   checks the 6-bit displacement range
//!         LM   BUF          ; LM/STM take only the address operand
//!         ORG  0x100        ; set location counter (halfword address)
//!         DC   H(-5)        ; halfword constant
//!         DC   F(0x12345678); fullword constant (2 halfwords)
//! ```
//!
//! Numbers are decimal or `0x` hex. The assembler chooses SRS over RS
//! automatically when the displacement fits (§2.2.3: "halfword
//! instructions are automatically selected by the assembler unless
//! otherwise specified"); a label used as a displacement forces RS.

use std::collections::HashMap;

#[derive(Debug)]
pub struct AsmError {
    pub line: usize,
    pub msg: String,
}

impl std::fmt::Display for AsmError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.msg)
    }
}

/// Assembled program: (halfword address, halfwords) chunks plus the entry
/// address (address of the first instruction assembled).
pub struct Program {
    pub chunks: Vec<(u16, Vec<u16>)>,
    pub entry: u16,
    /// Label -> halfword address, for tests that poke program data.
    pub labels: HashMap<String, u16>,
}

impl Program {
    pub fn label(&self, name: &str) -> Option<u32> {
        self.labels.get(name).map(|&a| a as u32)
    }

    pub fn load(&self, mem: &mut crate::mem::Memory) -> Result<(), crate::mem::AddressError> {
        for (addr, words) in &self.chunks {
            mem.load_halfwords(*addr as u32, words)?;
        }
        Ok(())
    }
}

// ---- instruction table ----

#[derive(Clone, Copy)]
enum R1Field {
    /// First operand is a register (or M1 mask) written by the programmer.
    Reg,
    /// R1 field is a fixed op-code extension; no first operand in source.
    Opx(u8),
}

#[derive(Clone, Copy)]
enum Kind {
    /// RR form only. `alt` selects the 11101 plane.
    Rr { op5: u8, alt: bool },
    /// Storage-operand instruction. `srs`: has an SRS form; `rs_marker` is
    /// the bits 8-12 pattern of its RS form (0b11110 or 0b11111);
    /// `full`: fullword operand (SRS displacement scaling).
    Mem { op5: u8, r1: R1Field, srs: bool, rs_marker: u8, full: bool },
    /// SI: SRS plus immediate halfword. All SI ops are op code 10110.
    Si { opx: u8 },
    /// RI: RR-immediate. All RI ops are op code 10110.
    Ri { opx: u8 },
    /// Shift: op5 (11110/11111) and type bits 14-15.
    Shift { op5: u8, ty: u8 },
    /// SRS-format relative branch: op5 and sub-op bits 14-15.
    RelBranch { op5: u8, subop: u8 },
    Lfxi,
}

fn table() -> &'static [(&'static str, Kind)] {
    use Kind::*;
    use R1Field::*;
    const T: &[(&str, Kind)] = &[
        // fixed point
        ("A", Mem { op5: 0b00000, r1: Reg, srs: true, rs_marker: 0b11110, full: true }),
        ("AH", Mem { op5: 0b10000, r1: Reg, srs: true, rs_marker: 0b11110, full: false }),
        ("AST", Mem { op5: 0b00000, r1: Reg, srs: false, rs_marker: 0b11111, full: true }),
        ("S", Mem { op5: 0b00001, r1: Reg, srs: true, rs_marker: 0b11110, full: true }),
        ("SH", Mem { op5: 0b10001, r1: Reg, srs: true, rs_marker: 0b11110, full: false }),
        ("SST", Mem { op5: 0b00001, r1: Reg, srs: false, rs_marker: 0b11111, full: true }),
        ("C", Mem { op5: 0b00010, r1: Reg, srs: true, rs_marker: 0b11110, full: true }),
        ("CH", Mem { op5: 0b10010, r1: Reg, srs: true, rs_marker: 0b11110, full: false }),
        ("D", Mem { op5: 0b01001, r1: Reg, srs: true, rs_marker: 0b11110, full: true }),
        ("M", Mem { op5: 0b01000, r1: Reg, srs: true, rs_marker: 0b11110, full: true }),
        ("MH", Mem { op5: 0b10101, r1: Reg, srs: true, rs_marker: 0b11110, full: false }),
        ("MIH", Mem { op5: 0b10011, r1: Reg, srs: false, rs_marker: 0b11111, full: false }),
        ("L", Mem { op5: 0b00011, r1: Reg, srs: true, rs_marker: 0b11110, full: true }),
        ("LH", Mem { op5: 0b10011, r1: Reg, srs: true, rs_marker: 0b11110, full: false }),
        ("LA", Mem { op5: 0b11101, r1: Reg, srs: true, rs_marker: 0b11110, full: false }),
        ("IAL", Mem { op5: 0b11100, r1: Reg, srs: true, rs_marker: 0b11111, full: false }),
        ("IHL", Mem { op5: 0b10000, r1: Reg, srs: false, rs_marker: 0b11111, full: false }),
        ("ST", Mem { op5: 0b00110, r1: Reg, srs: true, rs_marker: 0b11110, full: true }),
        ("STH", Mem { op5: 0b10111, r1: Reg, srs: true, rs_marker: 0b11110, full: false }),
        ("LM", Mem { op5: 0b11001, r1: Opx(0b100), srs: false, rs_marker: 0b11111, full: true }),
        ("STM", Mem { op5: 0b11001, r1: Opx(0b000), srs: false, rs_marker: 0b11111, full: true }),
        ("TD", Mem { op5: 0b10100, r1: Opx(0b000), srs: true, rs_marker: 0b11110, full: false }),
        ("ZH", Mem { op5: 0b10100, r1: Opx(0b001), srs: true, rs_marker: 0b11110, full: false }),
        ("SHW", Mem { op5: 0b10100, r1: Opx(0b010), srs: true, rs_marker: 0b11110, full: false }),
        ("TH", Mem { op5: 0b10100, r1: Opx(0b011), srs: true, rs_marker: 0b11110, full: false }),
        // logical storage forms
        ("N", Mem { op5: 0b00100, r1: Reg, srs: true, rs_marker: 0b11110, full: true }),
        ("O", Mem { op5: 0b00101, r1: Reg, srs: true, rs_marker: 0b11110, full: true }),
        ("X", Mem { op5: 0b01110, r1: Reg, srs: true, rs_marker: 0b11110, full: true }),
        ("NST", Mem { op5: 0b00100, r1: Reg, srs: false, rs_marker: 0b11111, full: true }),
        ("OST", Mem { op5: 0b00101, r1: Reg, srs: false, rs_marker: 0b11111, full: true }),
        ("XST", Mem { op5: 0b01110, r1: Reg, srs: false, rs_marker: 0b11111, full: true }),
        // RR
        ("AR", Rr { op5: 0b00000, alt: false }),
        ("SR", Rr { op5: 0b00001, alt: false }),
        ("CR", Rr { op5: 0b00010, alt: false }),
        ("LR", Rr { op5: 0b00011, alt: false }),
        ("NR", Rr { op5: 0b00100, alt: false }),
        ("OR", Rr { op5: 0b00101, alt: false }),
        ("XR", Rr { op5: 0b01110, alt: false }),
        ("MR", Rr { op5: 0b01000, alt: false }),
        ("DR", Rr { op5: 0b01001, alt: false }),
        ("XUL", Rr { op5: 0b00000, alt: true }),
        ("CBL", Rr { op5: 0b00001, alt: true }),
        ("LCR", Rr { op5: 0b11101, alt: true }),
        ("NCT", Rr { op5: 0b11100, alt: true }),
        ("SUM", Rr { op5: 0b10011, alt: true }),
        ("BALR", Rr { op5: 0b11100, alt: false }),
        ("BCR", Rr { op5: 0b11000, alt: false }),
        ("BCRE", Rr { op5: 0b11000, alt: true }),
        ("BCTR", Rr { op5: 0b11010, alt: false }),
        ("BVCR", Rr { op5: 0b11001, alt: false }),
        // branches, RS
        ("BAL", Mem { op5: 0b11100, r1: Reg, srs: false, rs_marker: 0b11110, full: false }),
        ("BC", Mem { op5: 0b11000, r1: Reg, srs: false, rs_marker: 0b11110, full: false }),
        ("BCT", Mem { op5: 0b11010, r1: Reg, srs: false, rs_marker: 0b11110, full: false }),
        ("BVC", Mem { op5: 0b11001, r1: Reg, srs: false, rs_marker: 0b11110, full: false }),
        ("BIX", Mem { op5: 0b11011, r1: Reg, srs: false, rs_marker: 0b11110, full: false }),
        // branches, SRS relative
        ("BCF", RelBranch { op5: 0b11011, subop: 0b00 }),
        ("BCB", RelBranch { op5: 0b11011, subop: 0b10 }),
        ("BCTB", RelBranch { op5: 0b11011, subop: 0b11 }),
        ("BVCF", RelBranch { op5: 0b11001, subop: 0b01 }),
        // RI / SI (all op code 10110)
        ("AHI", Ri { opx: 0b000 }),
        ("ZRB", Ri { opx: 0b001 }),
        ("OHI", Ri { opx: 0b010 }),
        ("TRB", Ri { opx: 0b011 }),
        ("XHI", Ri { opx: 0b100 }),
        ("CHI", Ri { opx: 0b101 }),
        ("NHI", Ri { opx: 0b110 }),
        ("MHI", Ri { opx: 0b111 }),
        ("MSTH", Si { opx: 0b000 }),
        ("ZB", Si { opx: 0b001 }),
        ("SB", Si { opx: 0b010 }),
        ("TB", Si { opx: 0b011 }),
        ("XIST", Si { opx: 0b100 }),
        ("CIST", Si { opx: 0b101 }),
        ("NIST", Si { opx: 0b110 }),
        // shifts
        ("SLL", Shift { op5: 0b11110, ty: 0b00 }),
        ("SRA", Shift { op5: 0b11110, ty: 0b01 }),
        ("SRL", Shift { op5: 0b11110, ty: 0b10 }),
        ("SRR", Shift { op5: 0b11110, ty: 0b11 }),
        ("SLDL", Shift { op5: 0b11111, ty: 0b00 }),
        ("SRDA", Shift { op5: 0b11111, ty: 0b01 }),
        ("SRDL", Shift { op5: 0b11111, ty: 0b10 }),
        ("SRDR", Shift { op5: 0b11111, ty: 0b11 }),
        ("LFXI", Lfxi),
    ];
    T
}

// ---- parsing ----

#[derive(Debug, Clone)]
enum Expr {
    Num(i64),
    Label(String),
}

impl Expr {
    fn eval(&self, labels: &HashMap<String, u16>, line: usize) -> Result<i64, AsmError> {
        match self {
            Expr::Num(n) => Ok(*n),
            Expr::Label(l) => labels
                .get(l)
                .map(|v| *v as i64)
                .ok_or_else(|| err(line, format!("undefined label {l}"))),
        }
    }
}

#[derive(Debug, Clone)]
enum AddrOperand {
    /// D2(B2) or D2(X2,B2); `ia`/`i` from mnemonic suffixes @ and #.
    Based { d: Expr, x: Option<u8>, b2: u8 },
    /// Bare displacement/label: RS extended with B2=11 (address = disp).
    Direct { d: Expr },
}

struct Stmt {
    line: usize,
    addr: u16,
    body: Body,
}

enum Body {
    Data(Vec<ExprSized>),
    Instr {
        kind: Kind,
        ia: bool,
        i_bit: bool,
        r1: Option<u8>,
        addr_op: Option<AddrOperand>,
        r2: Option<u8>,
        imm: Option<Expr>,
        /// resolved size in halfwords
        size: u16,
    },
}

struct ExprSized {
    e: Expr,
    halfwords: u16,
}

fn err(line: usize, msg: impl Into<String>) -> AsmError {
    AsmError { line, msg: msg.into() }
}

fn parse_num(s: &str, line: usize) -> Result<i64, AsmError> {
    let t = s.trim();
    let (neg, t) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t),
    };
    let v = if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        i64::from_str_radix(h, 16)
    } else {
        t.parse::<i64>()
    }
    .map_err(|_| err(line, format!("bad number '{s}'")))?;
    Ok(if neg { -v } else { v })
}

fn parse_expr(s: &str, line: usize) -> Result<Expr, AsmError> {
    let t = s.trim();
    if t.is_empty() {
        return Err(err(line, "empty operand"));
    }
    let first = t.chars().next().unwrap();
    if first.is_ascii_digit() || first == '-' {
        Ok(Expr::Num(parse_num(t, line)?))
    } else {
        Ok(Expr::Label(t.to_string()))
    }
}

fn parse_reg(s: &str, line: usize) -> Result<u8, AsmError> {
    let n = parse_num(s.trim(), line)?;
    if (0..=7).contains(&n) {
        Ok(n as u8)
    } else {
        Err(err(line, format!("register {n} out of range 0-7")))
    }
}

/// Parse `D`, `D(B)`, or `D(X,B)`.
fn parse_addr(s: &str, line: usize) -> Result<AddrOperand, AsmError> {
    let t = s.trim();
    if let Some(open) = t.find('(') {
        if !t.ends_with(')') {
            return Err(err(line, format!("bad address operand '{t}'")));
        }
        let d = parse_expr(&t[..open], line)?;
        let inner = &t[open + 1..t.len() - 1];
        let parts: Vec<&str> = inner.split(',').collect();
        match parts.len() {
            1 => Ok(AddrOperand::Based { d, x: None, b2: parse_b2(parts[0], line)? }),
            2 => Ok(AddrOperand::Based {
                d,
                x: Some(parse_reg(parts[0], line)?),
                b2: parse_b2(parts[1], line)?,
            }),
            _ => Err(err(line, format!("bad address operand '{t}'"))),
        }
    } else {
        Ok(AddrOperand::Direct { d: parse_expr(t, line)? })
    }
}

fn parse_b2(s: &str, line: usize) -> Result<u8, AsmError> {
    let n = parse_num(s.trim(), line)?;
    if (0..=3).contains(&n) {
        Ok(n as u8)
    } else {
        Err(err(line, format!("base register {n} out of range 0-3")))
    }
}

// ---- assembly ----

pub fn assemble(src: &str) -> Result<Program, AsmError> {
    let optable: HashMap<&str, Kind> = table().iter().cloned().collect();
    let mut labels: HashMap<String, u16> = HashMap::new();
    let mut stmts: Vec<Stmt> = Vec::new();
    let mut loc: u16 = 0;
    let mut entry: Option<u16> = None;

    // Pass 1: parse, size, place, collect labels.
    for (lineno, raw) in src.lines().enumerate() {
        let line = lineno + 1;
        let mut text = raw;
        if let Some(p) = text.find(';') {
            text = &text[..p];
        }
        if text.trim_start().starts_with('*') {
            continue;
        }
        let mut text = text.trim();
        // labels (possibly several, though one is typical)
        while let Some(colon) = text.find(':') {
            let name = text[..colon].trim();
            if name.is_empty() || name.contains(char::is_whitespace) {
                break;
            }
            if labels.insert(name.to_string(), loc).is_some() {
                return Err(err(line, format!("duplicate label {name}")));
            }
            text = text[colon + 1..].trim();
        }
        if text.is_empty() {
            continue;
        }
        let (mn, rest) = match text.find(char::is_whitespace) {
            Some(p) => (&text[..p], text[p..].trim()),
            None => (text, ""),
        };
        let mn_upper = mn.to_ascii_uppercase();

        if mn_upper == "ORG" {
            let v = parse_num(rest, line)?;
            if !(0..=0xFFFF).contains(&v) {
                return Err(err(line, "ORG out of range"));
            }
            loc = v as u16;
            continue;
        }
        if mn_upper == "DC" {
            let mut items = Vec::new();
            for item in split_top_level(rest) {
                let it = item.trim();
                let (sz, inner) = if let Some(x) = strip_wrap(it, 'H') {
                    (1u16, x)
                } else if let Some(x) = strip_wrap(it, 'F') {
                    (2u16, x)
                } else {
                    return Err(err(line, format!("DC operand '{it}' (use H(..) or F(..))")));
                };
                items.push(ExprSized { e: parse_expr(inner, line)?, halfwords: sz });
            }
            let sz: u16 = items.iter().map(|i| i.halfwords).sum();
            stmts.push(Stmt { line, addr: loc, body: Body::Data(items) });
            loc = loc.wrapping_add(sz);
            continue;
        }

        // mnemonic with optional addressing suffixes @ (IA) and # (I)
        let (base, ia, i_bit) = strip_suffixes(&mn_upper);
        let (base, alias_r1) = match base.as_str() {
            "B" => ("BC".to_string(), Some(0b111u8)),
            "NOP" => ("BCR".to_string(), Some(0b000u8)),
            _ => (base, None),
        };
        let kind = *optable
            .get(base.as_str())
            .ok_or_else(|| err(line, format!("unknown mnemonic '{mn}'")))?;

        let ops: Vec<String> = if rest.is_empty() {
            vec![]
        } else {
            split_top_level(rest).into_iter().map(|s| s.trim().to_string()).collect()
        };

        let mut r1 = alias_r1;
        let mut r2 = None;
        let mut addr_op = None;
        let mut imm = None;

        match kind {
            Kind::Rr { .. } => {
                match (r1, ops.len()) {
                    (Some(_), 0) => r2 = Some(0), // NOP
                    (Some(_), 1) => r2 = Some(parse_reg(&ops[0], line)?),
                    (None, 2) => {
                        r1 = Some(parse_reg(&ops[0], line)?);
                        r2 = Some(parse_reg(&ops[1], line)?);
                    }
                    _ => return Err(err(line, "RR form takes R1,R2")),
                }
            }
            Kind::Mem { r1: R1Field::Reg, .. } => {
                let need = if r1.is_some() { 1 } else { 2 };
                if ops.len() != need {
                    return Err(err(line, "expected R1,address"));
                }
                let mut idx = 0;
                if r1.is_none() {
                    r1 = Some(parse_reg(&ops[0], line)?);
                    idx = 1;
                }
                addr_op = Some(parse_addr(&ops[idx], line)?);
            }
            Kind::Mem { r1: R1Field::Opx(_), .. } => {
                if ops.len() != 1 {
                    return Err(err(line, "expected address operand"));
                }
                addr_op = Some(parse_addr(&ops[0], line)?);
            }
            Kind::Si { .. } => {
                if ops.len() != 2 {
                    return Err(err(line, "expected D2(B2),data"));
                }
                addr_op = Some(parse_addr(&ops[0], line)?);
                imm = Some(parse_expr(&ops[1], line)?);
            }
            Kind::Ri { .. } | Kind::Lfxi => {
                if ops.len() != 2 {
                    return Err(err(line, "expected R,data"));
                }
                r1 = Some(parse_reg(&ops[0], line)?);
                imm = Some(parse_expr(&ops[1], line)?);
            }
            Kind::Shift { .. } => {
                if ops.len() != 2 {
                    return Err(err(line, "expected R1,count"));
                }
                r1 = Some(parse_reg(&ops[0], line)?);
                imm = Some(parse_expr(&ops[1], line)?);
            }
            Kind::RelBranch { .. } => {
                if ops.len() != 2 {
                    return Err(err(line, "expected M1,target"));
                }
                r1 = Some(parse_reg(&ops[0], line)?);
                imm = Some(parse_expr(&ops[1], line)?);
            }
        }

        let size = size_of(&kind, ia, i_bit, &addr_op);
        if entry.is_none() {
            entry = Some(loc);
        }
        stmts.push(Stmt {
            line,
            addr: loc,
            body: Body::Instr { kind, ia, i_bit, r1, addr_op, r2, imm, size },
        });
        loc = loc.wrapping_add(size);
    }

    // Pass 2: encode.
    let mut chunks: Vec<(u16, Vec<u16>)> = Vec::new();
    for st in &stmts {
        let words = encode(st, &labels)?;
        match chunks.last_mut() {
            Some((a, ws)) if *a as u32 + ws.len() as u32 == st.addr as u32 => ws.extend(words),
            _ => chunks.push((st.addr, words)),
        }
    }
    Ok(Program { chunks, entry: entry.unwrap_or(0), labels })
}

fn strip_suffixes(mn: &str) -> (String, bool, bool) {
    let mut base = mn.to_string();
    let mut ia = false;
    let mut i_bit = false;
    loop {
        if base.ends_with('@') {
            ia = true;
            base.pop();
        } else if base.ends_with('#') {
            i_bit = true;
            base.pop();
        } else {
            break;
        }
    }
    (base, ia, i_bit)
}

fn strip_wrap(s: &str, c: char) -> Option<&str> {
    let s = s.trim();
    let mut chars = s.chars();
    if chars.next()? != c {
        return None;
    }
    let rest = &s[1..];
    if rest.starts_with('(') && rest.ends_with(')') {
        Some(&rest[1..rest.len() - 1])
    } else {
        None
    }
}

/// Split on commas not inside parentheses.
fn split_top_level(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut cur = String::new();
    for ch in s.chars() {
        match ch {
            '(' => {
                depth += 1;
                cur.push(ch);
            }
            ')' => {
                depth -= 1;
                cur.push(ch);
            }
            ',' if depth == 0 => {
                out.push(cur.clone());
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

/// Instruction size in halfwords. SRS is chosen when possible: literal
/// displacement fitting the 6-bit field (after fullword scaling), no index,
/// no indirect/automod, and the instruction has an SRS form.
fn size_of(kind: &Kind, ia: bool, i_bit: bool, addr_op: &Option<AddrOperand>) -> u16 {
    match kind {
        Kind::Rr { .. } | Kind::Shift { .. } | Kind::RelBranch { .. } | Kind::Lfxi => 1,
        Kind::Si { .. } | Kind::Ri { .. } => 2,
        Kind::Mem { srs, full, .. } => {
            if !srs || ia || i_bit {
                return 2;
            }
            match addr_op {
                Some(AddrOperand::Based { d: Expr::Num(n), x: None, b2: _ }) => {
                    let scale = if *full { 2 } else { 1 };
                    if *n >= 0 && *n % scale == 0 && (*n / scale) <= 55 {
                        1
                    } else {
                        2
                    }
                }
                _ => 2,
            }
        }
    }
}

fn encode(st: &Stmt, labels: &HashMap<String, u16>) -> Result<Vec<u16>, AsmError> {
    let line = st.line;
    match &st.body {
        Body::Data(items) => {
            let mut words = Vec::new();
            for it in items {
                let v = it.e.eval(labels, line)?;
                match it.halfwords {
                    1 => {
                        check_range(v, -0x8000, 0xFFFF, line)?;
                        words.push(v as u16);
                    }
                    _ => {
                        check_range(v, -0x8000_0000, 0xFFFF_FFFF, line)?;
                        words.push((v as u32 >> 16) as u16);
                        words.push(v as u32 as u16);
                    }
                }
            }
            Ok(words)
        }
        Body::Instr { kind, ia, i_bit, r1, addr_op, r2, imm, size } => {
            let r1v = r1.unwrap_or(0) as u16;
            match kind {
                Kind::Rr { op5, alt } => {
                    let marker = if *alt { 0b11101u16 } else { 0b11100 };
                    Ok(vec![
                        (*op5 as u16) << 11 | r1v << 8 | marker << 3 | r2.unwrap() as u16,
                    ])
                }
                Kind::Lfxi => {
                    let v = imm.as_ref().unwrap().eval(labels, line)?;
                    check_range(v, -2, 13, line)?;
                    let code = (v + 2) as u16;
                    Ok(vec![0b10111u16 << 11 | r1v << 8 | 0b1110 << 4 | code])
                }
                Kind::Shift { op5, ty } => {
                    let v = imm.as_ref().unwrap().eval(labels, line)?;
                    check_range(v, 0, 63, line)?;
                    Ok(vec![
                        (*op5 as u16) << 11 | r1v << 8 | (v as u16) << 2 | *ty as u16,
                    ])
                }
                Kind::RelBranch { op5, subop } => {
                    let target = imm.as_ref().unwrap().eval(labels, line)?;
                    // Displacement relative to the updated IC (§5.4/5.6);
                    // these are one-halfword instructions.
                    let updated = st.addr.wrapping_add(1) as i64;
                    let disp = match subop {
                        0b00 | 0b01 => target - updated, // forward
                        _ => updated - target,           // backward
                    };
                    if !(0..=55).contains(&disp) {
                        return Err(err(
                            line,
                            format!("relative branch displacement {disp} outside 0-55"),
                        ));
                    }
                    Ok(vec![
                        (*op5 as u16) << 11 | r1v << 8 | (disp as u16) << 2 | *subop as u16,
                    ])
                }
                Kind::Si { opx } => {
                    let (d, b2) = match addr_op {
                        Some(AddrOperand::Based { d, x: None, b2 }) => (d, *b2),
                        _ => return Err(err(line, "SI operand must be D2(B2)")),
                    };
                    let dv = d.eval(labels, line)?;
                    check_range(dv, 0, 55, line)?;
                    let immv = imm.as_ref().unwrap().eval(labels, line)?;
                    check_range(immv, -0x8000, 0xFFFF, line)?;
                    Ok(vec![
                        0b10110u16 << 11 | (*opx as u16) << 8 | (dv as u16) << 2 | b2 as u16,
                        immv as u16,
                    ])
                }
                Kind::Ri { opx } => {
                    let immv = imm.as_ref().unwrap().eval(labels, line)?;
                    check_range(immv, -0x8000, 0xFFFF, line)?;
                    Ok(vec![
                        0b10110u16 << 11 | (*opx as u16) << 8 | 0b11100 << 3 | r1v,
                        immv as u16,
                    ])
                }
                Kind::Mem { op5, r1: r1f, rs_marker, full, .. } => {
                    let r1field = match r1f {
                        R1Field::Reg => r1v,
                        R1Field::Opx(o) => *o as u16,
                    };
                    let ao = addr_op
                        .as_ref()
                        .ok_or_else(|| err(line, "missing address operand"))?;
                    if *size == 1 {
                        // SRS
                        let (d, b2) = match ao {
                            AddrOperand::Based { d, x: None, b2 } => (d, *b2),
                            _ => unreachable!(),
                        };
                        let dv = d.eval(labels, line)?;
                        let scale = if *full { 2 } else { 1 };
                        let dv = dv / scale;
                        Ok(vec![
                            (*op5 as u16) << 11 | r1field << 8 | (dv as u16) << 2 | b2 as u16,
                        ])
                    } else {
                        match ao {
                            AddrOperand::Direct { d } => {
                                // RS extended with B2=11: EA = displacement.
                                let dv = d.eval(labels, line)?;
                                check_range(dv, -0x8000, 0xFFFF, line)?;
                                Ok(vec![
                                    (*op5 as u16) << 11
                                        | r1field << 8
                                        | (*rs_marker as u16) << 3
                                        | 0b011,
                                    dv as u16,
                                ])
                            }
                            AddrOperand::Based { d, x: None, b2 } if !ia && !i_bit => {
                                // RS extended with base.
                                let dv = d.eval(labels, line)?;
                                check_range(dv, -0x8000, 0xFFFF, line)?;
                                Ok(vec![
                                    (*op5 as u16) << 11
                                        | r1field << 8
                                        | (*rs_marker as u16) << 3
                                        | *b2 as u16,
                                    dv as u16,
                                ])
                            }
                            AddrOperand::Based { d, x, b2 } => {
                                // RS indexed (AM=1).
                                let dv = d.eval(labels, line)?;
                                check_range(dv, 0, 0x7FF, line)?;
                                let xv = x.unwrap_or(0) as u16;
                                Ok(vec![
                                    (*op5 as u16) << 11
                                        | r1field << 8
                                        | (*rs_marker as u16) << 3
                                        | 0b100
                                        | *b2 as u16,
                                    xv << 13
                                        | (*ia as u16) << 12
                                        | (*i_bit as u16) << 11
                                        | dv as u16,
                                ])
                            }
                        }
                    }
                }
            }
        }
    }
}

fn check_range(v: i64, lo: i64, hi: i64, line: usize) -> Result<(), AsmError> {
    if v < lo || v > hi {
        Err(err(line, format!("value {v} outside range {lo}..{hi}")))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::{decode, Instr, Operand};

    #[test]
    fn assembles_and_decodes() {
        let p = assemble(
            "START: AR 3,5\n\
             \x20      A 2,10(1)\n\
             \x20      A 2,0x1234\n\
             \x20      AHI 4,-1\n\
             \x20      SLL 1,5\n\
             \x20      BC 7,START\n",
        )
        .unwrap();
        assert_eq!(p.entry, 0);
        let words = &p.chunks[0].1;
        // AR 3,5
        let d = decode(words[0], 0).unwrap();
        assert_eq!(d.instr, Instr::Ar);
        assert_eq!((d.r1, d.operand), (3, Operand::R(5)));
        // A 2,10(1): SRS, fullword scaling 10/2 = 5 in the field
        let d = decode(words[1], 0).unwrap();
        assert_eq!(d.instr, Instr::A);
        assert_eq!(d.operand, Operand::Srs { d: 5, b2: 1 });
        // A 2,0x1234: RS extended, B2=11
        let d = decode(words[2], words[3]).unwrap();
        assert_eq!(d.instr, Instr::A);
        assert_eq!(d.operand, Operand::RsExt { d16: 0x1234, b2: 3 });
        // AHI 4,-1
        let d = decode(words[4], words[5]).unwrap();
        assert_eq!(d.instr, Instr::Ahi);
        assert_eq!((d.r1, d.imm), (4, 0xFFFF));
        // SLL 1,5
        let d = decode(words[6], 0).unwrap();
        assert_eq!(d.instr, Instr::Sll);
        assert_eq!(d.operand, Operand::Count(5));
        // BC 7,0
        let d = decode(words[7], words[8]).unwrap();
        assert_eq!(d.instr, Instr::Bc);
        assert_eq!(d.operand, Operand::RsExt { d16: 0, b2: 3 });
    }

    #[test]
    fn srs_vs_rs_selection() {
        // Halfword op: displacement up to 55 fits SRS.
        let p = assemble("LH 1,55(2)\nLH 1,56(2)\n").unwrap();
        let w = &p.chunks[0].1;
        assert_eq!(decode(w[0], 0).unwrap().operand, Operand::Srs { d: 55, b2: 2 });
        assert_eq!(w.len(), 3); // second one is RS extended
        assert_eq!(
            decode(w[1], w[2]).unwrap().operand,
            Operand::RsExt { d16: 56, b2: 2 }
        );
        // Fullword op: displacement scaled by 2; odd displacements need RS.
        let p = assemble("L 1,110(2)\nL 1,111(2)\n").unwrap();
        let w = &p.chunks[0].1;
        assert_eq!(decode(w[0], 0).unwrap().operand, Operand::Srs { d: 55, b2: 2 });
        assert_eq!(
            decode(w[1], w[2]).unwrap().operand,
            Operand::RsExt { d16: 111, b2: 2 }
        );
    }

    #[test]
    fn indexed_and_indirect() {
        let p = assemble("L@ 1,16(0,1)\nL# 1,8(3,2)\nL 1,4(5,0)\n").unwrap();
        let w = &p.chunks[0].1;
        let d = decode(w[0], w[1]).unwrap();
        assert_eq!(
            d.operand,
            Operand::RsIdx { d11: 16, b2: 1, x: 0, ia: true, i: false }
        );
        let d = decode(w[2], w[3]).unwrap();
        assert_eq!(
            d.operand,
            Operand::RsIdx { d11: 8, b2: 2, x: 3, ia: false, i: true }
        );
        let d = decode(w[4], w[5]).unwrap();
        assert_eq!(
            d.operand,
            Operand::RsIdx { d11: 4, b2: 0, x: 5, ia: false, i: false }
        );
    }

    #[test]
    fn relative_branches() {
        // BCB back to a label 3 halfwords earlier; updated IC = addr+1.
        let p = assemble("TOP: AR 1,1\n AR 2,2\n AR 3,3\n BCB 7,TOP\n").unwrap();
        let w = &p.chunks[0].1;
        let d = decode(w[3], 0).unwrap();
        assert_eq!(d.instr, Instr::Bcb);
        // instruction at 3, updated IC 4, target 0 -> disp 4
        assert_eq!(d.operand, Operand::Srs { d: 4, b2: 0 });
    }

    #[test]
    fn data_and_org() {
        let p = assemble("ORG 0x20\nDC F(0x11223344),H(-2)\n").unwrap();
        assert_eq!(p.chunks[0].0, 0x20);
        assert_eq!(p.chunks[0].1, vec![0x1122, 0x3344, 0xFFFE]);
    }
}
