//! Simulated bus subsystems: data sources for flight code to consume.
//!
//! HONESTY NOTE — read this before trusting any number produced here.
//! Nothing in this file is recovered NASA hardware behaviour. These are
//! *simulated* devices: they answer the bus protocol faithfully enough
//! that genuine flight code runs against them, but every value they
//! return is invented by this project. The protocol shape (command
//! word, IUA addressing, data-word format) is the real one, per
//! App. III; the data is not telemetry and must never be presented as
//! though it were.
//!
//! Why simulate rather than emulate: the flight routines that read
//! these devices consume *numbers*. Reproducing an inertial measurement
//! unit's internals would take months and change nothing those routines
//! see. A device that answers with plausible values unlocks the code
//! paths immediately — which is the point.

use crate::gpc::BusSubsystem;
use crate::iop::BusWord;

/// A generic polled data source: answers a command addressed to its
/// interface-unit address with a fixed number of data words.
///
/// This is the shape most Shuttle sensors presented to the buses — the
/// GPC commands, the device replies with a burst of halfwords — so one
/// implementation serves an inertial unit, an air-data probe, or an
/// accelerometer package, differing only in what words it returns.
pub struct DataSource {
    pub bus: usize,
    pub iua: u8,
    /// The words this device reports, in order. Simulated values.
    pub words: Vec<u16>,
    /// Command opcode (bits 5-7 of the command word) this device
    /// answers; other opcodes are ignored, as a real terminal ignores
    /// traffic addressed elsewhere.
    pub opcode: u32,
    /// Bumped each poll so a program can see the source is live.
    pub polls: u32,
    /// Fault injection: when set, the device answers with corrupted
    /// validity bits — the failure a receiver's checks should catch.
    pub garble: bool,
}

impl DataSource {
    pub fn new(bus: usize, iua: u8, words: Vec<u16>) -> DataSource {
        DataSource { bus, iua, words, opcode: 1, polls: 0, garble: false }
    }

    /// An inertial unit reporting a body attitude as three angles, in
    /// the fixed-point form the flight code expects (binary point
    /// between bits 15 and 16, i.e. whole degrees in the upper half).
    pub fn imu(bus: usize, iua: u8, roll: i16, pitch: i16, yaw: i16) -> DataSource {
        DataSource::new(bus, iua, vec![roll as u16, pitch as u16, yaw as u16])
    }

    /// An air-data probe: altitude in feet, airspeed in knots, and a
    /// status word whose low bit marks the reading valid.
    pub fn air_data(bus: usize, iua: u8, altitude: u16, airspeed: u16) -> DataSource {
        DataSource::new(bus, iua, vec![altitude, airspeed, 0x0001])
    }
}

impl BusSubsystem for DataSource {
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
        if bus != self.bus || !w.cmd_sync {
            return Vec::new();
        }
        if (w.info >> 19) as u8 & 0x1F != self.iua || (w.info >> 16) & 7 != self.opcode {
            return Vec::new();
        }
        self.polls += 1;
        let count = (w.info & 0xFFFF).max(1) as usize;
        (0..count)
            .map(|i| {
                let v = self.words.get(i).copied().unwrap_or(0);
                let mut word = BusWord::data(self.iua, v);
                if self.garble {
                    word.info &= !7; // wreck the validity pattern
                }
                word
            })
            .collect()
    }
}

/// Mass memory: the tape units the crew loaded flight software from
/// between mission phases, because a megabyte could not hold a whole
/// flight. Answers reads from a simulated store.
///
/// SIMULATED: the block contents are whatever this project put there.
pub struct MassMemory {
    pub bus: usize,
    pub iua: u8,
    /// Named blocks, as a real load would be organised by major function.
    pub blocks: Vec<(String, Vec<u16>)>,
    pub selected: usize,
    pub loads: u32,
}

impl MassMemory {
    pub fn new(bus: usize, iua: u8) -> MassMemory {
        MassMemory { bus, iua, blocks: Vec::new(), selected: 0, loads: 0 }
    }

    pub fn with_block(mut self, name: &str, words: Vec<u16>) -> MassMemory {
        self.blocks.push((name.to_string(), words));
        self
    }
}

impl BusSubsystem for MassMemory {
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
        if bus != self.bus || !w.cmd_sync {
            return Vec::new();
        }
        if (w.info >> 19) as u8 & 0x1F != self.iua {
            return Vec::new();
        }
        match (w.info >> 16) & 7 {
            // opcode 3: select a block (a "major function" load)
            3 => {
                self.selected = (w.info & 0xFFFF) as usize % self.blocks.len().max(1);
                Vec::new()
            }
            // opcode 1: read `count` words of the selected block
            1 => {
                self.loads += 1;
                let count = (w.info & 0xFFFF).max(1) as usize;
                let empty = Vec::new();
                let block = self.blocks.get(self.selected).map(|b| &b.1).unwrap_or(&empty);
                (0..count)
                    .map(|i| BusWord::data(self.iua, block.get(i).copied().unwrap_or(0)))
                    .collect()
            }
            _ => Vec::new(),
        }
    }
}
