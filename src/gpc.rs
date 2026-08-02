//! A complete General Purpose Computer (CPU + IOP) and the multi-GPC
//! redundant set — phase 4.
//!
//! The Shuttle DPS ran four PASS GPCs as a synchronized redundant set
//! plus a BFS machine listening on the buses. Nothing here loads flight
//! software; the point is the *mechanisms*: N complete GPCs, a shared
//! serial-bus fabric where every GPC hears the others (App. III §4
//! listen mode), and fault injection so divergence and voting are
//! testable with our own software.

use crate::cpu::{Cpu, Trap};
use crate::iop::{BusFabric, BusWord, Iop, NUM_BCES};
use std::collections::VecDeque;

/// One GPC: an AP-101S CPU and its IOP sharing main storage.
pub struct Gpc {
    pub cpu: Cpu,
    pub iop: Iop,
}

impl Gpc {
    pub fn new(mem: crate::mem::Memory) -> Gpc {
        Gpc { cpu: Cpu::new(mem), iop: Iop::new() }
    }

    /// One time slice: one CPU instruction (skipped in the wait state),
    /// then one IOP slice (MSC + every busy BCE). An MSC @INT is routed
    /// to the CPU as external interrupt 2 (Figure 2-20: "Ext 2 IOP
    /// Programmed Interrupts", pending level index 4, mask bit 37).
    pub fn step(&mut self, buses: &mut dyn BusFabric) -> Result<(), Trap> {
        if !self.cpu.psw.wait {
            self.cpu.step()?;
        }
        self.iop.step(&mut self.cpu.mem, buses);
        if self.iop.cpu_interrupt.take().is_some() {
            self.cpu.pending_system[4] = true;
        }
        Ok(())
    }
}

/// N GPCs on a shared serial-bus fabric. A word transmitted by one GPC
/// on bus `b` is delivered to every other GPC's receive queue for bus
/// `b` — the flight-critical-bus arrangement that let each machine
/// listen to the others' traffic.
pub struct RedundantSet {
    pub gpcs: Vec<Gpc>,
    /// rx[gpc][bus]: words awaiting that GPC's receiver.
    rx: Vec<Vec<VecDeque<BusWord>>>,
    /// A GPC that trapped is dead (fail-silent) and no longer steps —
    /// the crudest fault model; richer ones inject at memory/bus level.
    pub dead: Vec<Option<Trap>>,
}

struct FabricView<'a> {
    me: usize,
    rx: &'a mut [Vec<VecDeque<BusWord>>],
}

impl BusFabric for FabricView<'_> {
    fn transmit(&mut self, bus: usize, w: BusWord) {
        for (g, q) in self.rx.iter_mut().enumerate() {
            if g != self.me {
                q[bus].push_back(w);
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
        }
    }

    /// Advance every live GPC one time slice.
    pub fn step(&mut self) {
        for (i, gpc) in self.gpcs.iter_mut().enumerate() {
            if self.dead[i].is_some() {
                continue;
            }
            let mut view = FabricView { me: i, rx: &mut self.rx };
            if let Err(t) = gpc.step(&mut view) {
                self.dead[i] = Some(t);
            }
        }
    }

    pub fn run(&mut self, steps: usize) {
        for _ in 0..steps {
            self.step();
        }
    }
}
