//! The redundant-set demo software image — shared by the phase-4 tests
//! and the `lazap-set` watchable demo. One identical program set (CPU
//! compute/poll/vote, MSC sequencer, BCE bus programs) configured only
//! by GPC id, the way one PASS image ran on four machines.

use crate::asm::assemble;
use crate::gpc::Gpc;
use crate::Memory;

pub const MBOX: u32 = 0x1000;

/// The common CPU program, parameterized only by the GPC id (the real
/// set used a config table; an immediate keeps the assembler simple).
pub fn cpu_program(id: u8) -> String {
    format!(
        "
        ORG  0x100
START:  LH   5,INA          ; sensor a
        AH   5,INB          ; + sensor b = my value
        LFXI 2,{id}         ; my GPC id (index register)
        LA   1,0x1000       ; mailbox base (GR1; B2=11 would mean
                            ; no-base in RS forms, section 2.2.8)
        STH  5,0(2,1)       ; publish my value to my mailbox
        LH   6,BUDGET       ; poll budget: a silent GPC must not stall
POLL:   AHI  6,-1           ; the set forever (timeout protocol)
        BC   2,VOTE         ; budget exhausted: vote with what we have
        LH   7,0(1)         ; wait until all four mailboxes are filled
        BC   4,POLL
        LH   7,1(1)
        BC   4,POLL
        LH   7,2(1)
        BC   4,POLL
        LH   7,3(1)
        BC   4,POLL
VOTE:   LFXI 4,0            ; fail mask
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
BUDGET: DC   H(200)
VERDICT: DC  H(0)
"
    )
}

/// MSC sequencer: poll own mailbox until the CPU publishes, then start
/// BCEs 1-4 (@SIO) and wait. Hand-encoded App. II forms.
pub fn msc_program(id: u32) -> ([u16; 8], u32) {
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
pub fn tx_program(i: u16) -> [u16; 7] {
    [
        0xF200, 0x1000, // #LBR 0x1000
        0xF640, 0x0800, // #CMDI: listen command (IOP addr 8, index 0)
        0x8000 | i,     // #TDS count 0, disp i (our mailbox)
        0xE800,         // #SIB
        0x0800,         // #WAT
    ]
}

/// Listener program for bus `j` (BCE j of every other GPC): base and
/// receive patience (#LTOI — the MTO register governs how long the BCE
/// waits for each input word, §3.4), then #WIX through a one-entry
/// branch table to a #RDS into mailbox j, flag, wait.
pub fn listener_program(j: u16, at: u32) -> [u16; 10] {
    let rds_at = (at + 4) as u16;
    [
        0xF200,
        0x1000,          // #LBR 0x1000
        0xB000 | 500,    // #LTOI 500: wait up to 500 slices per word
        0b00100_000_0000_0011, // #WIX disp 3 -> table at at+8
        0x6000 | j,      // #RDS count 0, disp j
        0xE800,          // #SIB
        0x0800,          // #WAT
        0,               // pad
        0, rds_at,       // table entry 0 (fullword) -> the #RDS
    ]
}

pub fn build_gpc(id: u8, faulty: bool) -> Gpc {
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
    {
        let mut iop = gpc.iop.borrow_mut();
        iop.halted = false;
        iop.msc.busy = true;
        iop.msc.pc = 0x140;
        for b in 0..4usize {
            iop.bces[b].pc = if b == id as usize {
                0x200 + 0x10 * b as u32
            } else {
                0x280 + 0x10 * b as u32
            };
        }
        iop.mia_xmtr_enable = 1 << (31 - id as u32);
        iop.mia_rcvr_enable = 0xF000_0000;
    }
    gpc
}
