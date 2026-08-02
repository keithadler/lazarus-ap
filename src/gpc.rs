//! A complete General Purpose Computer (CPU + IOP) and the multi-GPC
//! redundant set — phase 4.
//!
//! The Shuttle DPS ran four PASS GPCs as a synchronized redundant set
//! plus a BFS machine listening on the buses. Nothing here loads flight
//! software; the point is the *mechanisms*: N complete GPCs, a shared
//! serial-bus fabric where every GPC hears the others (App. III §4
//! listen mode), cross-wired discrete lines (the sync-discrete
//! arrangement), force-voting actuators, and fault injection so
//! divergence and failover are testable with our own software.

use crate::cpu::{Cpu, IoSubsystem, PcResponse, Trap};
use crate::iop::{BusFabric, BusWord, Iop, NUM_BCES};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

/// Adapter installing a shared IOP as the CPU's I/O subsystem, so CPU
/// software drives it with real PC instructions (PCO/PCI command words)
/// while the [`Gpc`] container can still step it.
struct IoBridge(Rc<RefCell<Iop>>);

impl IoSubsystem for IoBridge {
    fn pc(&mut self, cw: u32, data: Option<u32>) -> PcResponse {
        self.0.borrow_mut().pc(cw, data)
    }
}

/// One GPC: an AP-101S CPU and its IOP sharing main storage. The IOP is
/// reachable both from CPU software (PC instructions through the bridge)
/// and from the host (via [`Gpc::iop`]).
pub struct Gpc {
    pub cpu: Cpu,
    pub iop: Rc<RefCell<Iop>>,
}

impl Gpc {
    pub fn new(mem: crate::mem::Memory) -> Gpc {
        let iop = Rc::new(RefCell::new(Iop::new()));
        let mut cpu = Cpu::new(mem);
        cpu.io = Some(Box::new(IoBridge(iop.clone())));
        Gpc { cpu, iop }
    }

    /// One time slice: one CPU instruction (skipped in the wait state),
    /// then one IOP slice (MSC + every busy BCE). An MSC @INT is routed
    /// to the CPU as external interrupt 2 (Figure 2-20: "Ext 2 IOP
    /// Programmed Interrupts", pending level index 4, mask bit 37).
    pub fn step(&mut self, buses: &mut dyn BusFabric) -> Result<(), Trap> {
        if !self.cpu.psw.wait {
            self.cpu.step()?;
        }
        let mut iop = self.iop.borrow_mut();
        iop.step(&mut self.cpu.mem, buses);
        if iop.cpu_interrupt.take().is_some() {
            self.cpu.pending_system[4] = true;
        }
        Ok(())
    }
}

/// Anything attached to the serial buses besides GPCs: display units,
/// mass memory, MDMs... A subsystem sees every word on the fabric and
/// may answer; its response words go onto the same bus, heard by every
/// GPC (commander and listeners alike — how the BFS shadowed PASS's
/// display traffic).
pub trait BusSubsystem {
    fn observe(&mut self, bus: usize, w: BusWord) -> Vec<BusWord>;
    /// The bus this subsystem answers on (its responses are emitted
    /// there).
    fn bus(&self) -> usize;
    /// Downcast access for tests and front ends.
    fn as_any(&self) -> &dyn std::any::Any;
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

/// A hydraulic-style force-voted actuator port set: one command port per
/// flight-critical bus. Ports physically sum their force; a port whose
/// command persistently deviates from the voted output is bypassed —
/// the outlier loses. This models the *mechanism* (secondary-actuator
/// force voting); thresholds and dynamics are ours, not a NASA spec.
pub struct ForceVotedActuator {
    /// The subsystem address whose data words this actuator obeys.
    pub iua: u8,
    /// Last commanded value per port (port i listens on bus i).
    pub ports: [Option<i16>; 4],
    pub bypassed: [bool; 4],
    /// Deviation from the voted value (inclusive) beyond which a port is
    /// bypassed.
    pub tolerance: i32,
}

impl ForceVotedActuator {
    pub fn new(iua: u8, tolerance: i32) -> ForceVotedActuator {
        ForceVotedActuator {
            iua,
            ports: [None; 4],
            bypassed: [false; 4],
            tolerance,
        }
    }

    /// Bus tap: record data words addressed to this actuator. Words with
    /// an invalid SEV pattern are rejected, as a real interface unit
    /// rejects garbled transmissions (App. III Table 1.2 validity rules).
    fn observe(&mut self, bus: usize, w: BusWord) {
        if bus < 4
            && !w.cmd_sync
            && (w.info >> 19) as u8 == self.iua
            && w.info & 7 == 0b101
        {
            self.ports[bus] = Some((w.info >> 3) as u16 as i16);
        }
    }

    /// The force-voted surface position: the median of the active,
    /// non-bypassed port commands. Ports outside `tolerance` of the vote
    /// are latched bypassed (and the vote recomputed without them).
    pub fn output(&mut self) -> Option<i32> {
        loop {
            let mut active: Vec<(usize, i32)> = self
                .ports
                .iter()
                .enumerate()
                .filter_map(|(i, p)| {
                    (!self.bypassed[i]).then_some(())?;
                    p.map(|v| (i, v as i32))
                })
                .collect();
            if active.is_empty() {
                return None;
            }
            active.sort_by_key(|&(_, v)| v);
            let vote = active[active.len() / 2].1;
            let mut newly_bypassed = false;
            for &(i, v) in &active {
                if (v - vote).abs() > self.tolerance {
                    self.bypassed[i] = true;
                    newly_bypassed = true;
                }
            }
            if !newly_bypassed {
                return Some(vote);
            }
        }
    }
}

/// N GPCs on a shared serial-bus fabric. A word transmitted by one GPC
/// on bus `b` is delivered to every other GPC's receive queue for bus
/// `b` — the flight-critical-bus arrangement that let each machine
/// listen to the others' traffic. Actuators tap the same buses.
pub struct RedundantSet {
    pub gpcs: Vec<Gpc>,
    /// rx[gpc][bus]: words awaiting that GPC's receiver.
    rx: Vec<Vec<VecDeque<BusWord>>>,
    /// A GPC that trapped is dead (fail-silent) and no longer steps —
    /// the crudest fault model; richer ones inject at memory/bus level.
    pub dead: Vec<Option<Trap>>,
    pub actuators: Vec<ForceVotedActuator>,
    pub subsystems: Vec<Box<dyn BusSubsystem>>,
    /// Bus-level fault injection: data words transmitted on this bus are
    /// delivered with their SEV validity bits corrupted — a garbled
    /// transmission every receiver's validity checks will reject.
    pub corrupt_bus: Option<usize>,
}

struct FabricView<'a> {
    me: usize,
    rx: &'a mut [Vec<VecDeque<BusWord>>],
    actuators: &'a mut [ForceVotedActuator],
    subsystems: &'a mut [Box<dyn BusSubsystem>],
    corrupt_bus: Option<usize>,
}

impl BusFabric for FabricView<'_> {
    fn transmit(&mut self, bus: usize, w: BusWord) {
        let mut w = w;
        if self.corrupt_bus == Some(bus) && !w.cmd_sync {
            w.info &= !7; // garble the SEV validity pattern
        }
        for (g, q) in self.rx.iter_mut().enumerate() {
            if g != self.me {
                q[bus].push_back(w);
            }
        }
        for a in self.actuators.iter_mut() {
            a.observe(bus, w);
        }
        // Subsystem responses go out on the responder's bus, heard by
        // every GPC — the commander's receive and any listeners.
        for sub in self.subsystems.iter_mut() {
            for r in sub.observe(bus, w) {
                let rb = sub.bus();
                for q in self.rx.iter_mut() {
                    q[rb].push_back(r);
                }
            }
        }
    }

    fn receive(&mut self, bus: usize) -> Option<BusWord> {
        self.rx[self.me][bus].pop_front()
    }
}

impl RedundantSet {
    pub fn new(gpcs: Vec<Gpc>) -> RedundantSet {
        let n = gpcs.len();
        RedundantSet {
            gpcs,
            rx: (0..n)
                .map(|_| (0..NUM_BCES).map(|_| VecDeque::new()).collect())
                .collect(),
            dead: vec![None; n],
            actuators: Vec::new(),
            subsystems: Vec::new(),
            corrupt_bus: None,
        }
    }

    /// Advance one GPC one time slice (building block for skewed
    /// clocks: step machines at different rates to model oscillator
    /// drift).
    pub fn step_one(&mut self, i: usize) {
        if self.dead[i].is_some() {
            return;
        }
        let mut view = FabricView {
            me: i,
            rx: &mut self.rx,
            actuators: &mut self.actuators,
            subsystems: &mut self.subsystems,
            corrupt_bus: self.corrupt_bus,
        };
        if let Err(t) = self.gpcs[i].step(&mut view) {
            self.dead[i] = Some(t);
        }
    }

    /// Cross-wire the sync discretes: GPC i's discrete output appears as
    /// discrete input line i on every GPC (its own included, matching
    /// the loopback a machine sees of its own line).
    pub fn wire_discretes(&mut self) {
        let lines: u32 = self
            .gpcs
            .iter()
            .enumerate()
            .map(|(i, g)| (g.iop.borrow().discrete_out as u32) << (31 - i))
            .sum();
        for gpc in &self.gpcs {
            gpc.iop.borrow_mut().discrete_in = lines;
        }
    }

    /// Advance every live GPC one time slice, then rewire discretes.
    pub fn step(&mut self) {
        for i in 0..self.gpcs.len() {
            self.step_one(i);
        }
        self.wire_discretes();
    }

    pub fn run(&mut self, steps: usize) {
        for _ in 0..steps {
            self.step();
        }
    }
}
