//! HAL/S User Control Program — the ground-equipment I/O trap layer
//! (phase 6, core subset).
//!
//! HAL/S programs built by the recovered HALSFC compiler do WRITE/READ
//! through runtime stubs; the emulator host watches for the CPU's
//! instruction address landing on trap addresses (OUTRAP = IOINIT+0x11,
//! CNTRAP = IOINIT+0x40, INTRAP from the symbol table), performs the
//! I/O side effect by reading the IOCODE/IOBUF cells from main storage,
//! and lets the stub code run on. End-of-program is SVC 0x0015.
//! Mechanism and IOCODE dispatch verified against yaGPC2's halucp.c
//! (handle_output/halucp_check_trap/svc handling).
//!
//! Scope: output codes 9 (IOUT, 32-bit integer), 10 (HOUT, 16-bit
//! integer), and 13 (COUT, character string) plus the halt SVC —
//! enough for WRITE(6) of text and integers. Floats (11/12), bit
//! strings (8), input, paging/column formatting, and byte-parity with
//! yaGPC2's emit_field are later work. The COUT buffer layout (length
//! halfword then packed 8-bit characters) mirrors the documented BOUT
//! shape; verify against read_char_string when chasing parity.

use crate::cpu::{Cpu, Trap};
use crate::Halt;

/// End-of-program supervisor call (HAL/S-FC convention, yaGPC2
/// halucp.c: "SVC 0x0015 is HAL/S-FC's universal, successful
/// end-of-program").
pub const SVC_END: u16 = 0x0015;

/// Per-channel output-mechanism state (yaGPC2 halucp.h): a line buffer
/// addressed by column (so COLUMN/TAB can move backward before the
/// line commits), deferred positioning, and the WRITE-statement flags.
#[derive(Default, Clone)]
struct Chan {
    column: usize, // 1-based
    line_buf: Vec<u8>,
    first_field: bool,
    suppress_sep: bool,
    suppress_adv: bool,
    has_written: bool,
    /// (down_lines, to_col) applied lazily before the next field.
    deferred: Option<(u32, usize)>,
    line_number: usize,
    /// Whether any field has ever been flushed on this channel — gates
    /// SKIP's first-ever-WRITE suppression (yaGPC2 everEmittedField).
    ever_emitted: bool,
}

pub struct HalUcp {
    pub outrap: u32,
    pub cntrap: u32,
    pub intrap: u32,
    pub iocode_addr: u32,
    pub iobuf_addr: u32,
    /// Captured program output (host side of WRITE).
    pub output: String,
    /// Program input (host side of READ): prime before running.
    pub input: String,
    channel: usize,
    chans: Vec<Chan>,
    /// One-shot suppression of the next trap check: after a delivered
    /// input value the INTRAP stub itself must execute (yaGPC2
    /// halucp_is_trap_addr's skipTrap).
    skip_trap: bool,
    read_terminated: bool,
    input_pos: usize,
}

/// USA003090 defaults as implemented by yaGPC2: PAGED channels, 132
/// printable columns, 5-blank field separator, 66 lines per page.
const LINE_WIDTH: usize = 132;
const SEP_BLANKS: usize = 5;
const LINES_PER_PAGE: usize = 66;

impl HalUcp {
    /// Resolve everything from an lnk101 symbols JSON, the way yaGPC2's
    /// halucp_init_from_symbols does: the IOINIT *section* base gives
    /// OUTRAP (+0x11) and CNTRAP (+0x40); the INTRAP/IOCODE/IOBUF
    /// *symbols* give the rest. None if any required name is missing.
    pub fn from_symbols_json(json: &str) -> Option<HalUcp> {
        let sym = crate::fcm::Symbols::parse(json);
        Some(HalUcp::new(
            sym.section("IOINIT")?,
            sym.symbol("INTRAP")?,
            sym.symbol("IOCODE")?,
            sym.symbol("IOBUF")?,
        ))
    }

    /// Trap addresses per yaGPC2: OUTRAP/CNTRAP at fixed offsets from
    /// the IOINIT section base; INTRAP/IOCODE/IOBUF from the symbol
    /// table.
    pub fn new(ioinit_base: u32, intrap: u32, iocode: u32, iobuf: u32) -> HalUcp {
        HalUcp {
            outrap: ioinit_base + 0x11,
            cntrap: ioinit_base + 0x40,
            intrap,
            iocode_addr: iocode,
            iobuf_addr: iobuf,
            output: String::new(),
            input: String::new(),
            channel: 6,
            chans: vec![
                Chan { column: 1, line_number: 1, ..Chan::default() };
                256
            ],
            skip_trap: false,
            read_terminated: false,
            input_pos: 0,
        }
    }

    fn newline(&mut self, ch: usize) {
        let c = &mut self.chans[ch];
        if !c.line_buf.is_empty() {
            self.output.push_str(&String::from_utf8_lossy(&c.line_buf));
        }
        self.output.push('\n');
        let c = &mut self.chans[ch];
        c.line_buf.clear();
        c.line_number += 1;
        c.column = 1;
        if c.line_number > LINES_PER_PAGE {
            self.output.push('\u{c}');
            self.chans[ch].line_number = 1;
        }
    }

    /// Write `text` into the line buffer at 1-based `col`, space-padding
    /// any gap; overstrike, never truncate what follows.
    fn buf_write_at(&mut self, ch: usize, col: usize, text: &[u8]) {
        let c = &mut self.chans[ch];
        let start = col - 1;
        if start > c.line_buf.len() {
            c.line_buf.resize(start, b' ');
        }
        for (i, &b) in text.iter().enumerate() {
            if start + i < c.line_buf.len() {
                c.line_buf[start + i] = b;
            } else {
                c.line_buf.push(b);
            }
        }
    }

    fn flush_positioning(&mut self, ch: usize) {
        if let Some((down, to_col)) = self.chans[ch].deferred.take() {
            for _ in 0..down {
                self.newline(ch);
            }
            self.chans[ch].ever_emitted = true;
            self.chans[ch].column = to_col;
        }
    }

    /// Field emission with separator/wrap rules (yaGPC2 emit_field).
    fn emit_field(&mut self, text: &str, is_char: bool) {
        let ch = self.channel;
        self.flush_positioning(ch);
        let need_sep = !self.chans[ch].first_field && !self.chans[ch].suppress_sep;
        self.chans[ch].suppress_sep = false;
        let sep = if need_sep { SEP_BLANKS } else { 0 };
        let len = text.len();
        if !is_char {
            if self.chans[ch].column + sep + len - 1 > LINE_WIDTH {
                self.newline(ch);
                let col = self.chans[ch].column;
                self.buf_write_at(ch, col, text.as_bytes());
                self.chans[ch].column = col + len;
            } else {
                let col = self.chans[ch].column + sep;
                self.buf_write_at(ch, col, text.as_bytes());
                self.chans[ch].column = col + len;
            }
        } else {
            if sep > 0 {
                if self.chans[ch].column + sep > LINE_WIDTH + 1 {
                    self.newline(ch);
                } else {
                    self.chans[ch].column += sep;
                }
            }
            let mut pos = 0;
            while pos < len {
                if self.chans[ch].column > LINE_WIDTH {
                    self.newline(ch);
                }
                let remaining = LINE_WIDTH - self.chans[ch].column + 1;
                let take = remaining.min(len - pos).max(1);
                let col = self.chans[ch].column;
                self.buf_write_at(ch, col, &text.as_bytes()[pos..pos + take]);
                self.chans[ch].column += take;
                pos += take;
            }
        }
        self.chans[ch].first_field = false;
    }

    /// Flush any uncommitted line (end of run).
    pub fn flush(&mut self) {
        for ch in 0..self.chans.len() {
            if !self.chans[ch].line_buf.is_empty() {
                self.newline(ch);
            }
        }
    }

    /// Call with the CPU's next instruction address before each step:
    /// performs the host side of a trap hit. The stub instruction still
    /// executes (yaGPC2's check returns 'continue').
    pub fn check_trap(&mut self, cpu: &mut Cpu, nia: u32) {
        if self.skip_trap {
            self.skip_trap = false;
            return;
        }
        if nia == self.outrap {
            self.handle_output(cpu);
        } else if nia == self.cntrap {
            self.handle_control(cpu);
        } else if nia == self.intrap {
            self.handle_input(cpu);
        }
    }

    /// Next input field (yaGPC2 extract_next_field): skip whitespace;
    /// ';' terminates the READ, ',' is a null field, quoted strings for
    /// character/bit codes, otherwise a token up to comma/whitespace
    /// (with one trailing comma consumed).
    fn next_field(&mut self, iocode: u16) -> Option<Field> {
        let b = self.input.as_bytes();
        let mut i = self.input_pos;
        while i < b.len() && (b[i] as char).is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() {
            self.input_pos = i;
            return None; // exhausted: EOF
        }
        match b[i] {
            b';' => {
                self.input_pos = i + 1;
                Some(Field::Terminated)
            }
            b',' => {
                self.input_pos = i + 1;
                Some(Field::Null)
            }
            b'\'' if iocode == 13 || iocode == 8 => {
                let mut v = String::new();
                let mut p = i + 1;
                while p < b.len() {
                    if b[p] == b'\'' {
                        if p + 1 < b.len() && b[p + 1] == b'\'' {
                            v.push('\'');
                            p += 2;
                        } else {
                            p += 1;
                            break;
                        }
                    } else {
                        v.push(b[p] as char);
                        p += 1;
                    }
                }
                self.input_pos = p;
                self.consume_trailing_comma();
                Some(Field::Value(v))
            }
            _ => {
                let start = i;
                while i < b.len()
                    && b[i] != b','
                    && b[i] != b';'
                    && !(b[i] as char).is_ascii_whitespace()
                {
                    i += 1;
                }
                let v = self.input[start..i].to_string();
                self.input_pos = i;
                self.consume_trailing_comma();
                Some(Field::Value(v))
            }
        }
    }

    fn consume_trailing_comma(&mut self) {
        let b = self.input.as_bytes();
        let mut i = self.input_pos;
        while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
            i += 1;
        }
        if i < b.len() && b[i] == b',' {
            i += 1;
        }
        self.input_pos = i;
    }

    /// INTRAP: deliver the next input field into IOBUF per the input
    /// IOCODE (yaGPC2 handle_input/write_input_value), then let the
    /// stub execute once.
    fn handle_input(&mut self, cpu: &mut Cpu) {
        let iocode = cpu.mem.read_h(self.iocode_addr).unwrap_or(0);
        if self.read_terminated {
            self.skip_trap = true;
            return;
        }
        match self.next_field(iocode) {
            Some(Field::Terminated) => {
                self.read_terminated = true;
                self.skip_trap = true;
            }
            Some(Field::Null) => {
                // null field: target unchanged
                self.skip_trap = true;
            }
            None => {
                // EOF on input: error group 10, number 5 — dispatched to
                // an ON ERROR handler if one is installed, else the
                // program halts (yaGPC2 halucp_provide_eof).
                if self.try_on_error_dispatch(cpu, 10, 5) {
                    self.skip_trap = true;
                } else {
                    self.flush();
                    cpu.psw.wait = true;
                }
            }
            Some(Field::Value(text)) => {
                match iocode {
                    9 => {
                        let v = text.trim().parse::<i64>().unwrap_or(0);
                        let _ = cpu.mem.write_f(self.iobuf_addr, v as u32);
                    }
                    10 => {
                        let v = text.trim().parse::<i64>().unwrap_or(0);
                        let _ = cpu.mem.write_h(self.iobuf_addr, v as u16);
                    }
                    11 => {
                        let v = text.trim().parse::<f64>().unwrap_or(0.0);
                        let _ = cpu.mem.write_f(self.iobuf_addr, f64_to_ibm(v).0);
                    }
                    12 => {
                        let (hi, lo) = f64_to_ibm(text.trim().parse().unwrap_or(0.0));
                        let _ = cpu.mem.write_f(self.iobuf_addr, hi);
                        let _ = cpu.mem.write_f(self.iobuf_addr + 2, lo);
                    }
                    13 => {
                        // CIN: descriptor high byte = max length; write
                        // the new length and packed characters.
                        let desc = cpu.mem.read_h(self.iobuf_addr).unwrap_or(0);
                        let max = (desc >> 8) as usize;
                        let len = if max > 0 { text.len().min(max) } else { text.len() };
                        let _ = cpu
                            .mem
                            .write_h(self.iobuf_addr, ((max as u16) << 8) | len as u16);
                        let bytes = text.as_bytes();
                        for i in 0..len {
                            let hw_addr = self.iobuf_addr + 1 + (i as u32) / 2;
                            let old = cpu.mem.read_h(hw_addr).unwrap_or(0);
                            let nb = bytes[i] as u16;
                            let new = if i % 2 == 0 {
                                (nb << 8) | (old & 0xFF)
                            } else {
                                (old & 0xFF00) | nb
                            };
                            let _ = cpu.mem.write_h(hw_addr, new);
                        }
                    }
                    _ => {}
                }
                self.skip_trap = true;
            }
        }
    }

    /// ON ERROR dispatch (yaGPC2 try_on_error_dispatch): scan the
    /// call-frame chain's FIXV/handler slot pairs (direct case, slots at
    /// frame+18, walking caller frames via the saved R0), then the
    /// SCAL-frame-unwind case (slots at the caller's stack end, full
    /// R0-R7 + PSW word 0 restore). The IGNORE-AND-event tag values and
    /// the group-4/num-27 never-dispatch quirk are honored.
    fn try_on_error_dispatch(&mut self, cpu: &mut Cpu, group: u16, num: u16) -> bool {
        if group == 4 && num == 27 {
            return false;
        }
        let rd = |cpu: &Cpu, a: u32| cpu.mem.read_h(a & 0xFFFF).unwrap_or(0);
        // direct case, walking up
        let mut walk_sa = cpu.r(0) >> 16;
        let mut walk_r0 = cpu.r(0);
        for level in 0..8 {
            for slot in 0..16u32 {
                let off = 18 + slot * 2;
                let fixv = rd(cpu, walk_sa + off);
                if matches!(fixv >> 12, 0xF | 0x7 | 0xB) {
                    continue; // IGNORE-AND-event dispositions: no jump
                }
                if !match_error_handler(fixv, group, num) {
                    continue;
                }
                // Replicate the AEXIT epilogue the jump bypasses: restore
                // the caller's data-base registers from the matched frame.
                if level > 0 {
                    cpu.set_r(0, walk_r0);
                }
                let r1 = (rd(cpu, walk_sa + 5) as u32) << 16;
                let r3 = (rd(cpu, walk_sa + 9) as u32) << 16;
                cpu.set_r(1, r1);
                cpu.set_r(3, r3);
                cpu.psw.ic = rd(cpu, walk_sa + off + 1);
                return true;
            }
            let hi = rd(cpu, walk_sa + 2) as u32;
            let lo = rd(cpu, walk_sa + 3) as u32;
            if hi == 0 || hi == walk_sa {
                break;
            }
            let _ = level;
            walk_r0 = (hi << 16) | lo;
            walk_sa = hi;
        }
        // SCAL-frame-unwind case: the erroring RTL routine was SCAL-
        // called; recover the caller's R0 from the frame's save slots.
        let sa = cpu.r(0) >> 16;
        let hi = rd(cpu, sa + 2) as u32;
        let lo = rd(cpu, sa + 3) as u32;
        let stack_end = (hi + lo) & 0xFFFF;
        let fixv = rd(cpu, stack_end.wrapping_sub(2));
        let handler = rd(cpu, stack_end.wrapping_sub(1));
        if !match_error_handler(fixv, group, num) {
            return false;
        }
        let w0 = ((rd(cpu, sa) as u32) << 16) | rd(cpu, sa + 1) as u32;
        for i in 0..8u8 {
            let rh = rd(cpu, sa + 2 + i as u32 * 2) as u32;
            let rl = rd(cpu, sa + 2 + i as u32 * 2 + 1) as u32;
            cpu.set_r(i, (rh << 16) | rl);
        }
        cpu.psw.set_word0(w0);
        cpu.psw.ic = handler;
        true
    }

    fn handle_output(&mut self, cpu: &Cpu) {
        let iocode = cpu.mem.read_h(self.iocode_addr).unwrap_or(0);
        let text = match iocode {
            9 => {
                let v = cpu.mem.read_f(self.iobuf_addr).unwrap_or(0) as i32;
                format_integer(v as i64)
            }
            10 => {
                let v = cpu.mem.read_h(self.iobuf_addr).unwrap_or(0) as i16;
                format_integer(v as i64)
            }
            11 => {
                // EOUT: single-precision IBM hex float (§8 format).
                let w = cpu.mem.read_f(self.iobuf_addr).unwrap_or(0);
                format_scalar(ibm_to_f64(crate::float::unpack_short(w)), 7, 14)
            }
            12 => {
                // DOUT: double-precision (register-pair layout).
                let hi = cpu.mem.read_f(self.iobuf_addr).unwrap_or(0);
                let lo = cpu.mem.read_f(self.iobuf_addr + 2).unwrap_or(0);
                format_scalar(ibm_to_f64(crate::float::unpack_long(hi, lo)), 16, 23)
            }
            13 => {
                // Descriptor halfword: current length in the low byte
                // (max length in the high byte); characters packed two
                // per halfword. AP-101S DEU ASCII encoding: 0x00 = '"',
                // 0x16 = '_' (yaGPC2 read_char_string).
                let len = (cpu.mem.read_h(self.iobuf_addr).unwrap_or(0) & 0xFF) as u32;
                let mut s = String::new();
                for i in 0..len {
                    let hw = cpu
                        .mem
                        .read_h(self.iobuf_addr + 1 + i / 2)
                        .unwrap_or(0);
                    let b = if i % 2 == 0 { (hw >> 8) as u8 } else { hw as u8 };
                    s.push(match b {
                        0x00 => '"',
                        0x16 => '_',
                        0x20..=0x7E => b as char,
                        _ => '.',
                    });
                }
                s
            }
            other => format!("[IOCODE={other}?]"),
        };
        self.emit_field(&text, iocode == 13);
    }

    fn handle_control(&mut self, cpu: &Cpu) {
        // Control codes (yaGPC2 handle_control): 0/1 = READ IOINIT
        // (input unsupported here), 2/3 = WRITE IOINIT, 4 = LINE,
        // 5 = COLUMN (absolute), 6 = TAB (relative, signed).
        let iocode = cpu.mem.read_h(self.iocode_addr).unwrap_or(0);
        let mut param = cpu.mem.read_h(self.iobuf_addr).unwrap_or(0) as i32;
        if iocode == 6 && param & 0x8000 != 0 {
            param -= 0x10000;
        }
        match iocode {
            0 | 1 => {
                self.channel = param as usize & 0xFF;
                self.read_terminated = false;
            }
            2 | 3 => {
                let ch = param as usize & 0xFF;
                self.channel = ch;
                if self.chans[ch].deferred.is_some() {
                    self.flush_positioning(ch);
                }
                if !self.chans[ch].has_written {
                    self.chans[ch].has_written = true;
                    self.chans[ch].suppress_adv = false;
                    let col = self.chans[ch].column;
                    self.chans[ch].deferred = Some((0, col));
                } else if self.chans[ch].suppress_adv {
                    self.chans[ch].suppress_adv = false;
                    let col = self.chans[ch].column;
                    self.chans[ch].deferred = Some((0, col));
                } else {
                    self.chans[ch].deferred = Some((1, 1));
                }
                self.chans[ch].first_field = true;
                self.chans[ch].suppress_sep = false;
            }
            4 => {
                // LINE(n), paged: forward to line n, wrapping the page.
                let ch = self.channel;
                let cur = self.chans[ch].line_number as i32;
                let delta = if param >= cur {
                    param - cur
                } else {
                    (LINES_PER_PAGE as i32 - cur) + param
                };
                if let Some((_, col)) = self.chans[ch].deferred {
                    self.chans[ch].deferred = Some((delta as u32, col));
                } else {
                    for _ in 0..delta {
                        self.newline(ch);
                    }
                }
            }
            5 => {
                // COLUMN(n), absolute; lazy placement via the deferred
                // slot when one is pending.
                let ch = self.channel;
                let col = (param.max(1)) as usize;
                match self.chans[ch].deferred {
                    Some((down, _)) => self.chans[ch].deferred = Some((down, col)),
                    None => self.chans[ch].column = col,
                }
                self.chans[ch].suppress_sep = true;
            }
            6 => {
                // TAB(n), relative (signed).
                let ch = self.channel;
                match self.chans[ch].deferred {
                    Some((down, col)) => {
                        let t = (col as i32 + param).max(1) as usize;
                        self.chans[ch].deferred = Some((down, t));
                    }
                    None => {
                        let t = (self.chans[ch].column as i32 + param).max(1) as usize;
                        self.chans[ch].column = t;
                    }
                }
                self.chans[ch].suppress_sep = true;
            }
            7 => {
                // PAGE(n): n pages of line advances.
                let ch = self.channel;
                if param > 0 {
                    let down = param as u32 * LINES_PER_PAGE as u32;
                    match self.chans[ch].deferred {
                        Some((_, col)) => self.chans[ch].deferred = Some((down, col)),
                        None => {
                            for _ in 0..down {
                                self.newline(ch);
                            }
                        }
                    }
                }
            }
            8 => {
                // SKIP(n). A device's true first-ever WRITE performs
                // only initial positioning — the runtime's own row-
                // separator SKIP is suppressed until any field has ever
                // been emitted (yaGPC2's everEmittedField gate).
                let ch = self.channel;
                if param >= 0 {
                    match self.chans[ch].deferred {
                        Some((_, col)) if self.chans[ch].ever_emitted => {
                            self.chans[ch].deferred = Some((param as u32, col));
                        }
                        Some(_) => {}
                        None => {
                            for _ in 0..param {
                                self.newline(ch);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// FIXV matching (yaGPC2 match_error_handler): 0 = empty, 63 = bare
/// catch-all, TAG=0 user DO-blocks with 6-bit group/number fields
/// (0x3F wildcards).
fn match_error_handler(fixv: u16, group: u16, num: u16) -> bool {
    if fixv == 0 {
        return false;
    }
    if fixv == 63 {
        return true;
    }
    if (fixv >> 12) & 0xF != 0 {
        return false;
    }
    let num_field = (fixv >> 6) & 0x3F;
    let grp_field = fixv & 0x3F;
    (grp_field == 0x3F || grp_field == group) && (num_field == 0x3F || num_field == num)
}

enum Field {
    Value(String),
    Null,
    Terminated,
}

/// f64 -> IBM hexadecimal float (long form; short is the high word).
/// Truncating, like the hardware conversions.
fn f64_to_ibm(v: f64) -> (u32, u32) {
    if v == 0.0 {
        return (0, 0);
    }
    let neg = v < 0.0;
    let mut av = v.abs();
    let mut ch: i32 = 64;
    while av >= 1.0 {
        av /= 16.0;
        ch += 1;
    }
    while av < 1.0 / 16.0 {
        av *= 16.0;
        ch -= 1;
    }
    let frac = (av * (1u64 << 56) as f64) as u64; // 14 hex digits
    let hi = ((neg as u32) << 31)
        | ((ch.clamp(0, 127) as u32) << 24)
        | ((frac >> 32) as u32 & 0x00FF_FFFF);
    (hi, frac as u32)
}

fn ibm_to_f64(u: crate::float::Unpacked) -> f64 {
    if u.is_zero() {
        return 0.0;
    }
    // value = frac * 16^-14 * 16^(ch-64), frac being the 56-bit fraction
    let m = u.frac as f64 * (16f64).powi(u.ch - 78);
    if u.neg {
        -m
    } else {
        m
    }
}

/// Integers right-justify in an 11-column field (yaGPC2 format_integer).
fn format_integer(v: i64) -> String {
    format!("{v:>11}")
}

/// HAL/S scalar output format (yaGPC2 format_scalar): sign column,
/// one integer digit, `frac_digits` decimals, two-digit signed
/// exponent. Zero prints as " 0.0" left-justified in the field width.
fn format_scalar(v: f64, frac_digits: usize, total_width: usize) -> String {
    if v == 0.0 {
        return format!("{:<total_width$}", " 0.0");
    }
    let sign = if v < 0.0 { '-' } else { ' ' };
    let av = v.abs();
    let mut exp = av.log10().floor() as i32;
    let mut mant = av / 10f64.powi(exp);
    if mant >= 10.0 {
        mant /= 10.0;
        exp += 1;
    } else if mant < 1.0 {
        mant *= 10.0;
        exp -= 1;
    }
    let mut ms = format!("{mant:.frac_digits$}");
    if ms.find('.') != Some(1) {
        mant /= 10.0;
        exp += 1;
        ms = format!("{mant:.frac_digits$}");
    }
    let es = if exp >= 0 { '+' } else { '-' };
    format!("{sign}{ms}E{es}{:02}", exp.abs())
}

/// Outcome of running a HAL/S-style program to completion.
#[derive(Debug, PartialEq, Eq)]
pub enum HalRun {
    /// SVC 0x0015: successful end of program.
    Done,
    Halt(Halt),
    Steps,
}

/// Drive a CPU with UCP trapping until end-of-program, a halt, or the
/// step budget runs out.
///
/// SVC protocol (yaGPC2 halucp_handle_svc): the SVC's effective address
/// POINTS at the code — `svcCode = mem[ea]`. 0x0015 ends the program;
/// other codes are runtime builtins (events, error queries), treated as
/// no-ops in this core subset (the SVC has no PSA handler installed, so
/// skipping the failed swap and continuing IS the no-op).
pub fn run_hal(cpu: &mut Cpu, ucp: &mut HalUcp, max_steps: usize) -> HalRun {
    run_hal_traced(cpu, ucp, max_steps, |_, _, _| {})
}

/// `run_hal` with a per-step observer (called before each step with the
/// CPU, the UCP, and the 19-bit next-instruction address) — the hook the
/// trace generator uses.
pub fn run_hal_traced(
    cpu: &mut Cpu,
    ucp: &mut HalUcp,
    max_steps: usize,
    mut observe: impl FnMut(&Cpu, &HalUcp, u32),
) -> HalRun {
    for _ in 0..max_steps {
        let nia = cpu.expand_branch(cpu.psw.ic);
        observe(cpu, ucp, nia);
        ucp.check_trap(cpu, nia);
        match cpu.step() {
            Ok(_) => {}
            Err(Trap::UninitializedInterrupt { code, .. }) => {
                let ea = ((cpu.psw.ea_high as u32) << 15) | (code as u32 & 0x7FFF);
                let svc = cpu.mem.read_h(ea).unwrap_or(0);
                if svc == SVC_END {
                    ucp.flush();
                    return HalRun::Done;
                }
                // other builtin SVC: continue (no-op)
            }
            Err(t) => return HalRun::Halt(Halt::Trap(t)),
        }
        if cpu.psw.wait {
            return HalRun::Halt(Halt::Wait);
        }
    }
    HalRun::Steps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asm::assemble;
    use crate::{Cpu, Memory};

    /// A hand-built stand-in for a HALSFC-compiled program: runtime
    /// stubs at the trap addresses, WRITE protocol through IOCODE/IOBUF,
    /// SVC 0x15 to end. (Validation against a real HALSFC image is the
    /// next step; this proves the trap/protocol plumbing.)
    #[test]
    fn write_integer_and_string_then_end() {
        let ioinit = 0x600u32;
        let iocode = 0x700u32;
        let iobuf = 0x702u32;
        let src = format!(
            "
        ORG  0x100
        ; WRITE(6) 'HI THERE' -- COUT
        LFXI 1,13
        STH  1,{iocode}
        BAL  7,OUTSTUB
        ; WRITE(6) 4242 -- IOUT (fullword at IOBUF)
        LFXI 1,9
        STH  1,{iocode}
        BAL  7,OUTSTUB
        DC   H(0xC9FB)      ; SVC ENDCODE (EA points at the 0x0015 code)
        DC   H(ENDCODE)
ENDCODE: DC  H(0x0015)
        ; runtime output stub at OUTRAP (= IOINIT+0x11): return to caller
        ORG  {stub}
OUTSTUB: BCR 7,7
",
            iocode = iocode,
            stub = ioinit + 0x11,
        );
        let prog = assemble(&src).unwrap();
        let mut cpu = Cpu::new(Memory::new(0x2000));
        prog.load(&mut cpu.mem).unwrap();
        // COUT buffer: length 8, then packed characters
        cpu.mem.write_h(iobuf, 8).unwrap();
        for (i, pair) in b"HI THERE".chunks(2).enumerate() {
            cpu.mem
                .write_h(iobuf + 1 + i as u32, ((pair[0] as u16) << 8) | pair[1] as u16)
                .unwrap();
        }
        // IOUT value shares IOBUF; written before the second call by the
        // host here (a real program would store it itself).
        cpu.psw.ic = 0x100;
        let mut ucp = HalUcp::new(ioinit, 0, iocode, iobuf);
        // run first WRITE, then swap the buffer to the integer
        // (single-buffer protocol: one value per trap)
        let r = run_hal_with_swap(&mut cpu, &mut ucp, iobuf);
        assert_eq!(r, HalRun::Done);
        // Field layout per the ported emit_field: with no IOINIT in this
        // synthetic program, both fields take the default 5-blank
        // separator; the integer right-justifies in its 11-column field.
        assert_eq!(ucp.output, "     HI THERE            4242\n");
    }

    fn run_hal_with_swap(cpu: &mut Cpu, ucp: &mut HalUcp, iobuf: u32) -> HalRun {
        let mut swapped = false;
        for _ in 0..500 {
            let nia = cpu.expand_branch(cpu.psw.ic);
            if nia == ucp.outrap && !swapped && cpu.mem.read_h(ucp.iocode_addr).unwrap() == 9 {
                cpu.mem.write_f(iobuf, 4242).unwrap();
                swapped = true;
            }
            ucp.check_trap(cpu, nia);
            match cpu.step() {
                Ok(_) => {}
                Err(Trap::UninitializedInterrupt { code, .. }) => {
                    let ea =
                        ((cpu.psw.ea_high as u32) << 15) | (code as u32 & 0x7FFF);
                    if cpu.mem.read_h(ea).unwrap_or(0) == SVC_END {
                        ucp.flush();
                        return HalRun::Done;
                    }
                }
                Err(t) => return HalRun::Halt(Halt::Trap(t)),
            }
        }
        HalRun::Steps
    }
}
