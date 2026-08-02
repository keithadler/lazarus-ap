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
use lazarus_ap::gpc::{Gpc, RedundantSet};
use lazarus_ap::Memory;

const MBOX: u32 = 0x1000;

/// The common CPU program, parameterized only by the GPC id (the real
/// set used a config table; an immediate keeps the assembler simple).
fn cpu_program(id: u8) -> String {
    format!(
        "
        ORG  0x100
START:  LH   5,INA          ; sensor a
        AH   5,INB          ; + sensor b = my value
        LFXI 2,{id}         ; my GPC id (index register)
        LA   1,0x1000       ; mailbox base (GR1; B2=11 would mean
                            ; no-base in RS forms, section 2.2.8)
        STH  5,0(2,1)       ; publish my value to my mailbox
POLL:   LH   6,0(1)         ; wait until all four mailboxes are filled
        BC   4,POLL
        LH   6,1(1)
        BC   4,POLL
        LH   6,2(1)
        BC   4,POLL
        LH   6,3(1)
        BC   4,POLL
        LFXI 4,0            ; fail mask
        CH   5,0(1)         ; vote: compare my value against each GPC
        BC   4,SK0
        AHI  4,1            ; GPC 0 disagrees
SK0:    CH   5,1(1)
        BC   4,SK1
        AHI  4,2
SK1:    CH   5,2(1)
        BC   4,SK2
        AHI  4,4
SK2:    CH   5,3(1)
        BC   4,SK3
        AHI  4,8
SK3:    STH  4,VERDICT
DONE:   B    DONE
INA:    DC   H(40)
INB:    DC   H(2)
VERDICT: DC  H(0)
"
    )
}

/// MSC sequencer: poll own mailbox until the CPU publishes, then start
/// BCEs 1-4 (@SIO) and wait. Hand-encoded App. II forms.
fn msc_program(id: u32) -> ([u16; 8], u32) {
    let mbox = MBOX + id;
    (
        [
            0xF408,                 // @LH (long, T=1) ...
            mbox as u16,            //   own mailbox
            0x24FD,                 // @BC ACC=0 -> back 3 (keep polling)
            0xC000,                 // @DLY 0 (pad to even boundary)
            0xF400,                 // @LF ...
            0x0150,                 //   BCE start mask constant
            0xE400,                 // @SIO
            0x0800,                 // @WAT
        ],
        0x0140,
    )
}

/// Transmitter program for bus `i` (BCE i of GPC i): set base, send the
/// listen command, transmit our mailbox halfword, flag done, wait.
fn tx_program(i: u16) -> [u16; 7] {
    [
        0xF200, 0x1000, // #LBR 0x1000
        0xF640, 0x0800, // #CMDI: listen command (IOP addr 8, index 0)
        0x8000 | i,     // #TDS count 0, disp i (our mailbox)
        0xE800,         // #SIB
        0x0800,         // #WAT
    ]
}

/// Listener program for bus `j` (BCE j of every other GPC): base, #WIX
/// through a one-entry branch table to a #RDS into mailbox j, flag, wait.
/// Returns (image, wix table entry target offset).
fn listener_program(j: u16, at: u32) -> [u16; 10] {
    let rds_at = (at + 3) as u16;
    [
        0xF200,
        0x1000,          // #LBR 0x1000
        0b00100_000_0000_0101, // #WIX disp 5 -> table at at+8
        0x6000 | j,      // #RDS count 0, disp j
        0xE800,          // #SIB
        0x0800,          // #WAT
        0, 0,            // pad
        0, rds_at,       // table entry 0 (fullword) -> the #RDS
    ]
}

fn build_gpc(id: u8, faulty: bool) -> Gpc {
    let mut mem = Memory::new(0x4000);
    // CPU program
    let prog = assemble(&cpu_program(id)).expect("assembles");
    prog.load(&mut mem).unwrap();
    // fault injection: corrupt this GPC's sensor input (INA 40 -> 11)
    if faulty {
        let ina = prog.label("INA").unwrap();
        mem.write_h(ina, 11).unwrap();
    }
    // MSC program + constant
    let (msc, msc_at) = msc_program(id as u32);
    mem.load_halfwords(msc_at, &msc).unwrap();
    mem.write_f(0x150, 0x7800_0000).unwrap(); // start BCEs 1-4
    // BCE programs: transmitters at 0x200+0x10*i, listeners at 0x280+0x10*j
    for i in 0..4u16 {
        mem.load_halfwords(0x200 + 0x10 * i as u32, &tx_program(i)).unwrap();
        let at = 0x280 + 0x10 * i as u32;
        mem.load_halfwords(at, &listener_program(i, at)).unwrap();
    }

    let mut gpc = Gpc::new(mem);
    gpc.cpu.psw.ic = 0x100;
    // IOP configuration (the CPU would do this via PCO in a fuller
    // model): processor enabled, MSC busy at its program, BCE PCs set —
    // own bus transmitter, listeners elsewhere. MIA enables per §4.1:
    // transmitter only on our own bus, receivers on all four.
    gpc.iop.halted = false;
    gpc.iop.msc.busy = true;
    gpc.iop.msc.pc = 0x140;
    for b in 0..4usize {
        gpc.iop.bces[b].pc = if b == id as usize {
            0x200 + 0x10 * b as u32
        } else {
            0x280 + 0x10 * b as u32
        };
    }
    gpc.iop.mia_xmtr_enable = 1 << (31 - id as u32);
    gpc.iop.mia_rcvr_enable = 0xF000_0000;
    gpc
}

#[test]
fn four_gpcs_vote_out_the_faulty_one() {
    let mut set = RedundantSet::new(
        (0..4).map(|id| build_gpc(id, id == 2)).collect(),
    );
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
        for b in 0..4 {
            assert!(gpc.iop.bces[b].indicator, "GPC {i} BCE {b} done");
            assert!(!gpc.iop.bces[b].error, "GPC {i} BCE {b} clean");
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
}

#[test]
fn a_dead_gpc_is_outvoted_too() {
    // Kill GPC 1 outright (its CPU program is garbage -> it traps and
    // never publishes). The others must not hang on its silence forever
    // in a real system; here the poll loop would spin, so this test
    // documents today's behavior: fail-silent GPCs stall the exchange,
    // which is exactly why the real DPS needed sync/timeout protocols —
    // the next phase. We verify the fail-silence itself.
    let mut gpcs: Vec<Gpc> = (0..4).map(|id| build_gpc(id, false)).collect();
    gpcs[1].cpu.mem.write_h(0x100, 0b00110_001_11100_010).unwrap(); // ST has no RR form: illegal
    let mut set = RedundantSet::new(gpcs);
    set.run(50);
    assert!(set.dead[1].is_some(), "GPC 1 trapped");
    for i in [0usize, 2, 3] {
        assert!(set.dead[i].is_none(), "GPC {i} unaffected");
    }
}
