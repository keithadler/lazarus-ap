//! The phase-4 demonstration: four GPCs running IDENTICAL software — CPU
//! code, MSC sequencer, BCE bus programs — exchange their computed
//! values over shared serial buses using listen mode (App. III §4) and
//! each votes on the others. One GPC gets a corrupted sensor input; the
//! three healthy machines independently flag exactly that GPC, and the
//! sick machine flags everyone else (the classic split-vote signature).
//!
//! No flight software is involved: the redundancy-management logic is
//! ours. What is faithful is the machinery it runs on: AP-101S CPUs,
//! MSC/BCE programs from main storage, listen-mode bus exchange, and a
//! common software image configured only by GPC id — the way the real
//! DPS ran one PASS image on four machines.
//!
//! Memory map (identical in every GPC):
//!   0x0100  CPU program (compute, poll, vote)
//!   0x0140  MSC program (wait for own value, then @SIO the BCEs)
//!   0x0150  MSC constant: busy/wait mask for BCEs 1-4
//!   0x0200+ BCE programs: 4 transmitter variants + 4 listener variants
//!   0x1000  mailboxes: halfword slot per GPC (own written by CPU,
//!           others filled from the buses)
//!
//! Bus plan: GPC i commands bus i (transmitter BCE i); every other GPC's
//! BCE i sits in listen mode on bus i.

use lazarus_ap::asm::assemble;
use lazarus_ap::demo::{build_gpc, cpu_program, MBOX};
use lazarus_ap::gpc::{ForceVotedActuator, Gpc, RedundantSet};
use lazarus_ap::Memory;

#[test]
fn four_gpcs_vote_out_the_faulty_one() {
    let mut set = RedundantSet::new(
        (0..4).map(|id| build_gpc(id, id == 2)).collect(),
    );
    // A force-voted actuator taps the four buses (the values the GPCs
    // broadcast double as its port commands; IUA 8 is the demo's
    // subsystem address).
    set.actuators.push(ForceVotedActuator::new(8, 5));
    set.run(400);

    // no GPC trapped
    for (i, d) in set.dead.iter().enumerate() {
        assert!(d.is_none(), "GPC {i} died: {d:?}");
    }
    let verdict_addr = assemble(&cpu_program(0)).unwrap().label("VERDICT").unwrap();
    for (i, gpc) in set.gpcs.iter().enumerate() {
        // every GPC assembled the full mailbox picture over the buses
        let boxes: Vec<u16> = (0..4)
            .map(|j| gpc.cpu.mem.read_h(MBOX + j).unwrap())
            .collect();
        assert_eq!(boxes, vec![42, 42, 13, 42], "GPC {i} mailboxes");
        // all four BCEs finished their bus programs cleanly
        let iop = gpc.iop.borrow();
        for b in 0..4 {
            assert!(iop.bces[b].indicator, "GPC {i} BCE {b} done");
            assert!(!iop.bces[b].error, "GPC {i} BCE {b} clean");
        }
        let verdict = gpc.cpu.mem.read_h(verdict_addr).unwrap();
        if i == 2 {
            // the sick machine votes against the world
            assert_eq!(verdict, 0b1011, "GPC 2 flags GPCs 0, 1, 3");
        } else {
            // healthy machines independently reach the same verdict
            assert_eq!(verdict, 0b0100, "GPC {i} flags exactly GPC 2");
        }
    }
    // The actuator force-votes the same way: the outlier port is
    // bypassed and the surface follows the healthy command.
    let act = &mut set.actuators[0];
    assert_eq!(act.output(), Some(42));
    assert_eq!(act.bypassed, [false, false, true, false], "port 2 bypassed");
}

#[test]
fn a_dead_gpc_is_outvoted_too() {
    // Kill GPC 1 outright (an illegal instruction: it traps, goes
    // fail-silent, and never publishes). The survivors' poll budget
    // expires and they vote with what they have: GPC 1's empty mailbox
    // disagrees with everyone, so all three flag exactly GPC 1.
    let mut gpcs: Vec<Gpc> = (0..4).map(|id| build_gpc(id, false)).collect();
    gpcs[1].cpu.mem.write_h(0x100, 0b00110_001_11100_010).unwrap(); // ST has no RR form: illegal
    let mut set = RedundantSet::new(gpcs);
    set.run(4000);
    assert!(set.dead[1].is_some(), "GPC 1 trapped");
    let verdict_addr = assemble(&cpu_program(0)).unwrap().label("VERDICT").unwrap();
    for i in [0usize, 2, 3] {
        assert!(set.dead[i].is_none(), "GPC {i} unaffected");
        let gpc = &set.gpcs[i];
        let boxes: Vec<u16> = (0..4)
            .map(|j| gpc.cpu.mem.read_h(MBOX + j).unwrap())
            .collect();
        assert_eq!(boxes, vec![42, 0, 42, 42], "GPC {i} mailboxes");
        let verdict = gpc.cpu.mem.read_h(verdict_addr).unwrap();
        assert_eq!(verdict, 0b0010, "GPC {i} flags exactly the silent GPC 1");
    }
}

#[test]
fn sync_discretes_barrier() {
    // The sync-discrete mechanism: each GPC raises its discrete output
    // with a real PCO (PC instruction, DISCRETE OUTPUT SET), then polls
    // the cross-wired discrete inputs (PCI "D.I.A") until all four
    // lines are up — a hardware barrier, from software.
    fn barrier_program(id: u8) -> String {
        format!(
            "
        ORG  0x100
        LFXI 6,{id}         ; stagger: sick of lockstep? each GPC
DLY:    AHI  6,-1           ; arrives at the barrier at a different time
        BC   1,DLY
        L    2,CWSET
        DC   H(0xD9EA)      ; PC 1,2 - PCO: raise our discrete line
        L    2,CWDIA
WAITL:  DC   H(0xD9EA)      ; PC 1,2 - PCI: read the discrete inputs
        C    1,ALLUP
        BC   2,WAITL        ; not all up yet (less-than: high bits clear)
        LFXI 5,1
        STH  5,MARK         ; through the barrier
DONE:   B    DONE
CWSET:  DC   F(0x85100000)
CWDIA:  DC   F(0x08180000)
ALLUP:  DC   F(0xF0000000)
MARK:   DC   H(0)
"
        )
    }
    let mut gpcs = Vec::new();
    for id in 0..4u8 {
        let mut mem = lazarus_ap::Memory::new(0x2000);
        let prog = assemble(&barrier_program(id)).unwrap();
        prog.load(&mut mem).unwrap();
        let mut gpc = Gpc::new(mem);
        gpc.cpu.psw.ic = 0x100;
        gpcs.push(gpc);
    }
    let mut set = RedundantSet::new(gpcs);
    let mark = assemble(&barrier_program(0)).unwrap().label("MARK").unwrap();
    set.run(120);
    for (i, gpc) in set.gpcs.iter().enumerate() {
        assert_eq!(
            gpc.cpu.mem.read_h(mark).unwrap(),
            1,
            "GPC {i} passed the barrier"
        );
        assert_eq!(gpc.iop.borrow().discrete_in, 0xF000_0000);
    }
}

#[test]
fn bfs_style_fifth_listener_shadows_the_set() {
    // A fifth GPC joins with every BCE in listen mode and no transmitter
    // — the Backup Flight System arrangement: it hears everything, says
    // nothing. Running the SAME software image (id 4), it assembles the
    // full mailbox picture from the buses and reaches the same verdict
    // as the healthy PASS machines, while remaining invisible to them.
    let mut gpcs: Vec<Gpc> = (0..4).map(|id| build_gpc(id, id == 2)).collect();
    let bfs = build_gpc(4, false);
    bfs.iop.borrow_mut().mia_xmtr_enable = 0; // pure listener
    gpcs.push(bfs);
    let mut set = RedundantSet::new(gpcs);
    set.run(500);

    let verdict_addr = assemble(&cpu_program(0)).unwrap().label("VERDICT").unwrap();
    // The BFS shadowed the exchange and votes with the majority.
    let bfs = &set.gpcs[4];
    let boxes: Vec<u16> = (0..4)
        .map(|j| bfs.cpu.mem.read_h(MBOX + j).unwrap())
        .collect();
    assert_eq!(boxes, vec![42, 42, 13, 42], "BFS heard all four values");
    assert_eq!(
        bfs.cpu.mem.read_h(verdict_addr).unwrap(),
        0b0100,
        "BFS reaches the majority verdict"
    );
    // ...and the PASS machines never heard from it.
    for i in 0..4 {
        let gpc = &set.gpcs[i];
        assert_eq!(
            gpc.cpu.mem.read_h(MBOX + 4).unwrap(),
            0,
            "GPC {i}: no trace of the BFS on the buses"
        );
        assert_eq!(
            gpc.cpu.mem.read_h(verdict_addr).unwrap(),
            if i == 2 { 0b1011 } else { 0b0100 }
        );
    }
}

#[test]
fn garbled_bus_is_indistinguishable_from_a_sick_gpc() {
    // Corrupt the SEV validity bits of everything GPC 2 transmits — the
    // machine is HEALTHY, its bus is not. Every listener's validity
    // checks reject the garbled words (App. III Table 1.2), mailbox 2
    // never fills, and the set votes GPC 2 out anyway: at the receiver,
    // a bad bus and a bad GPC look identical. The actuator likewise
    // rejects the garbled commands and follows the healthy vote.
    let mut set = RedundantSet::new(
        (0..4).map(|id| build_gpc(id, false)).collect(),
    );
    set.corrupt_bus = Some(2);
    set.actuators.push(ForceVotedActuator::new(8, 5));
    set.run(4000);

    let verdict_addr = assemble(&cpu_program(0)).unwrap().label("VERDICT").unwrap();
    for i in [0usize, 1, 3] {
        let gpc = &set.gpcs[i];
        assert_eq!(
            gpc.cpu.mem.read_h(MBOX + 2).unwrap(),
            0,
            "GPC {i}: garbled words rejected, mailbox 2 empty"
        );
        assert_eq!(
            gpc.cpu.mem.read_h(verdict_addr).unwrap(),
            0b0100,
            "GPC {i} flags GPC 2"
        );
        // the listener BCE on bus 2 recorded the SEV validity error
        let iop = gpc.iop.borrow();
        assert!(iop.bces[2].error, "GPC {i} BCE 2 error-terminated");
        assert_ne!(iop.bces[2].status & (0b111 << 20), 0, "SEV recorded");
    }
    // GPC 2 itself heard everyone else fine, agrees with them (it is
    // healthy!), and flags NOBODY: a machine cannot see its own bus
    // fault from reception alone — which is why the real DPS
    // cross-strapped status and let the majority's view prevail.
    assert_eq!(set.gpcs[2].cpu.mem.read_h(verdict_addr).unwrap(), 0b0000);
    let act = &mut set.actuators[0];
    assert_eq!(act.ports[2], None, "actuator rejected garbled commands");
    assert_eq!(act.output(), Some(42));
}

#[test]
fn clock_skew_is_absorbed_by_the_buses() {
    // GPC 3 runs at quarter speed (oscillator drift, exaggerated). The
    // serial buses buffer its late transmissions and the poll budgets
    // absorb the wait: the set still converges on the same verdicts.
    let mut set = RedundantSet::new(
        (0..4).map(|id| build_gpc(id, id == 2)).collect(),
    );
    for tick in 0..4000usize {
        for i in 0..4 {
            if i != 3 || tick % 4 == 0 {
                set.step_one(i);
            }
        }
        set.wire_discretes();
    }
    let verdict_addr = assemble(&cpu_program(0)).unwrap().label("VERDICT").unwrap();
    for i in 0..4 {
        let boxes: Vec<u16> = (0..4)
            .map(|j| set.gpcs[i].cpu.mem.read_h(MBOX + j).unwrap())
            .collect();
        assert_eq!(boxes, vec![42, 42, 13, 42], "GPC {i} mailboxes");
        assert_eq!(
            set.gpcs[i].cpu.mem.read_h(verdict_addr).unwrap(),
            if i == 2 { 0b1011 } else { 0b0100 },
            "GPC {i} verdict unaffected by skew"
        );
    }
}
