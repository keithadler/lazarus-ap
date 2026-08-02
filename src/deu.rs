//! Display Electronics Unit — the crew interface (phase 5).
//!
//! On the Shuttle, the crew's keyboards and CRTs connected to Display
//! Electronics Units, which sat on the display buses as bus terminal
//! units: the GPCs polled them for keystrokes and sent them display
//! updates, all as ordinary serial-bus transactions (NASA DPS Overview
//! Workbook; see docs/DEU_STATUS.md).
//!
//! What is *sourced* here: the architecture (DEU as a polled bus
//! subsystem addressed by IUA; keystrokes travel the display bus to the
//! GPC; display data travels back), and the keyboard's key set (the
//! 32-key DPS keyboard: hex digits 0-9/A-F, +, -, ., OPS, SPEC, ITEM,
//! EXEC, PRO, RESUME, CLEAR, SYS SUMM, FAULT SUMM, GPC/CRT, I/O RESET,
//! ACK — Shuttle Crew Operations Manual / DPS Overview Workbook).
//!
//! What is EMULATOR CONVENTION (clearly not from a primary source — the
//! DEU's actual wire protocol is not in the documents recovered so
//! far): the command opcodes and keystroke/display word encodings
//! below. They are designed to be replaced if the real protocol
//! surfaces; the GPC side speaks them through ordinary BCE #MIN/#MOUT
//! instructions, which *is* faithful.

use crate::gpc::BusSubsystem;
use crate::iop::BusWord;
use std::collections::VecDeque;

/// DPS keyboard key codes (EMULATOR CONVENTION; the key *set* is the
/// documented 32-key DPS keyboard).
pub mod key {
    // hex digits 0-9, A-F encode as their value 0x00..0x0F
    pub const PLUS: u8 = 0x10;
    pub const MINUS: u8 = 0x11;
    pub const DOT: u8 = 0x12;
    pub const OPS: u8 = 0x13;
    pub const SPEC: u8 = 0x14;
    pub const ITEM: u8 = 0x15;
    pub const EXEC: u8 = 0x16;
    pub const PRO: u8 = 0x17;
    pub const RESUME: u8 = 0x18;
    pub const CLEAR: u8 = 0x19;
    pub const SYS_SUMM: u8 = 0x1A;
    pub const FAULT_SUMM: u8 = 0x1B;
    pub const GPC_CRT: u8 = 0x1C;
    pub const IO_RESET: u8 = 0x1D;
    pub const ACK: u8 = 0x1E;
}

/// DEU command opcodes, carried in bits 5-7 of the 24-bit command word
/// after the IUA (EMULATOR CONVENTION).
const OP_POLL_KEYS: u32 = 1;
const OP_DISPLAY_WRITE: u32 = 2;

/// A Display Electronics Unit on a display bus: buffers crew keystrokes
/// for the GPC to poll, and maintains a text screen the GPC writes.
pub struct Deu {
    /// Display bus this DEU answers on.
    pub bus: usize,
    /// Interface unit address the GPC commands it by.
    pub iua: u8,
    /// Keystrokes typed by the crew, oldest first.
    pub keys: VecDeque<u8>,
    /// The CRT: `rows x cols` characters (halfword text cells, written
    /// by GPC display-write messages at a running cursor).
    pub screen: Vec<u16>,
    pub cols: usize,
    cursor: usize,
    /// Words still expected as data for an in-progress display write.
    pending_write: usize,
}

impl Deu {
    /// A DEU with the historical 26x51-character CRT format scaled to
    /// `rows`/`cols` of the caller's choosing.
    pub fn new(bus: usize, iua: u8, rows: usize, cols: usize) -> Deu {
        Deu {
            bus,
            iua,
            keys: VecDeque::new(),
            screen: vec![b' ' as u16; rows * cols],
            cols,
            cursor: 0,
            pending_write: 0,
        }
    }

    /// Crew keystroke (the keyboard hardware debounced and queued them).
    pub fn press(&mut self, k: u8) {
        self.keys.push_back(k);
    }

    pub fn type_keys(&mut self, ks: &[u8]) {
        for &k in ks {
            self.press(k);
        }
    }

    /// The screen as text lines (for tests and front ends).
    pub fn screen_text(&self) -> Vec<String> {
        self.screen
            .chunks(self.cols)
            .map(|row| {
                row.iter()
                    .map(|&c| char::from_u32(c as u32).unwrap_or(' '))
                    .collect()
            })
            .collect()
    }
}

impl BusSubsystem for Deu {
    fn bus(&self) -> usize {
        self.bus
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn observe(&mut self, bus: usize, w: BusWord) -> Vec<BusWord> {
        if bus != self.bus {
            return Vec::new();
        }
        if w.cmd_sync {
            let iua = (w.info >> 19) as u8 & 0x1F;
            if iua != self.iua {
                return Vec::new();
            }
            let op = (w.info >> 16) & 7;
            let arg = w.info & 0xFFFF;
            match op {
                OP_POLL_KEYS => {
                    // Respond with `arg` keystroke words (0 = key buffer
                    // empty), as data words carrying our IUA.
                    (0..arg)
                        .map(|_| {
                            let k = self.keys.pop_front().unwrap_or(0xFF);
                            BusWord::data(self.iua, k as u16)
                        })
                        .collect()
                }
                OP_DISPLAY_WRITE => {
                    // `arg` low bits: cursor (character cell) in bits
                    // 4-15, data word count follows in the stream.
                    self.cursor = (arg >> 4) as usize % self.screen.len();
                    self.pending_write = (arg & 0xF) as usize;
                    Vec::new()
                }
                _ => Vec::new(),
            }
        } else {
            // Data words: consumed while a display write is pending and
            // the word carries our IUA.
            if self.pending_write > 0 && (w.info >> 19) as u8 & 0x1F == self.iua {
                let ch = (w.info >> 3) as u16;
                let at = self.cursor % self.screen.len();
                self.screen[at] = ch;
                self.cursor = (self.cursor + 1) % self.screen.len();
                self.pending_write -= 1;
            }
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_returns_keystrokes_then_empty_markers() {
        let mut deu = Deu::new(4, 0x0C, 4, 16);
        deu.type_keys(&[key::OPS, 2, 0, 1, key::PRO]);
        // GPC polls for 8 keystrokes
        let cmd = BusWord::command((0x0C << 19) | (OP_POLL_KEYS << 16) | 8);
        let resp = deu.observe(4, cmd);
        assert_eq!(resp.len(), 8);
        let codes: Vec<u16> = resp.iter().map(|w| (w.info >> 3) as u16).collect();
        assert_eq!(
            codes,
            vec![
                key::OPS as u16,
                2,
                0,
                1,
                key::PRO as u16,
                0xFF,
                0xFF,
                0xFF
            ]
        );
        // wrong IUA: silence
        let other = BusWord::command((0x0B << 19) | (OP_POLL_KEYS << 16) | 4);
        assert!(deu.observe(4, other).is_empty());
    }

    #[test]
    fn display_write_paints_the_screen() {
        let mut deu = Deu::new(4, 0x0C, 2, 8);
        // write 4 chars at cell 8 (row 1, col 0)
        let cmd = BusWord::command((0x0C << 19) | (OP_DISPLAY_WRITE << 16) | (8 << 4) | 4);
        deu.observe(4, cmd);
        for ch in b"PASS" {
            deu.observe(4, BusWord::data(0x0C, *ch as u16));
        }
        assert_eq!(deu.screen_text(), vec!["        ", "PASS    "]);
    }
}
