//! Flight code reading from simulated devices.
//!
//! The values are invented by this project; the bus protocol and the
//! code consuming them are not. A GPC polls an inertial unit and an
//! air-data probe over the serial buses and lands the readings in main
//! storage, exactly as it would from real hardware.

use lazarus_ap::gpc::{Gpc, RedundantSet};
use lazarus_ap::subsystems::{DataSource, MassMemory};
use lazarus_ap::Memory;

const BUF: u32 = 0x1900;

/// A GPC whose BCE polls `count` words from IUA 0x0E on `bus`.
fn poller(bus: usize, count: u16) -> Gpc {
    let mut mem = Memory::new(0x4000);
    let cmd: u32 = (0x0E << 19) | (1 << 16) | count as u32;
    mem.load_halfwords(
        0x200,
        &[
            0xF200, BUF as u16,       // #LBR
            0xB000 | 300,             // #LTOI
            0xC000,                   // pad to even
            0b11110001_00000000,      // #MIN
            count - 1,                //   transfer count
            (cmd >> 16) as u16,
            cmd as u16,
            0xE800,                   // #SIB when the burst lands
            0x0800,                   // #WAT
        ],
    )
    .unwrap();
    let mut gpc = Gpc::new(mem);
    gpc.cpu.psw.wait = true;
    {
        let mut iop = gpc.iop.borrow_mut();
        iop.halted = false;
        iop.bces[bus].busy = true;
        iop.bces[bus].pc = 0x200;
        iop.mia_xmtr_enable = 1 << (31 - bus as u32);
        iop.mia_rcvr_enable = 1 << (31 - bus as u32);
    }
    gpc
}

#[test]
fn gpc_reads_an_inertial_unit() {
    let mut set = RedundantSet::new(vec![poller(7, 3)]);
    // simulated attitude: roll -12, pitch +5, yaw +130 degrees
    set.subsystems
        .push(Box::new(DataSource::imu(7, 0x0E, -12, 5, 130)));
    set.run(300);

    let g = &set.gpcs[0];
    let read = |i: u32| g.cpu.mem.read_h(BUF + i).unwrap() as i16;
    assert_eq!([read(0), read(1), read(2)], [-12, 5, 130], "attitude words");
    assert!(g.iop.borrow().bces[7].indicator, "poll completed");
    assert!(!g.iop.borrow().bces[7].error, "no bus error");
    let src = set.subsystems[0]
        .as_any()
        .downcast_ref::<DataSource>()
        .unwrap();
    assert_eq!(src.polls, 1, "device was polled once");
}

#[test]
fn air_data_and_a_garbled_sensor() {
    // A healthy probe: 41,000 feet at 250 knots, status valid.
    let mut set = RedundantSet::new(vec![poller(7, 3)]);
    set.subsystems
        .push(Box::new(DataSource::air_data(7, 0x0E, 41000, 250)));
    set.run(300);
    let g = &set.gpcs[0];
    assert_eq!(g.cpu.mem.read_h(BUF).unwrap(), 41000, "altitude");
    assert_eq!(g.cpu.mem.read_h(BUF + 1).unwrap(), 250, "airspeed");
    assert_eq!(g.cpu.mem.read_h(BUF + 2).unwrap() & 1, 1, "valid flag");

    // The same probe with its validity bits corrupted: the receiver's
    // own checks reject it and the transfer error-terminates, rather
    // than the flight code consuming garbage.
    let mut set = RedundantSet::new(vec![poller(7, 3)]);
    let mut bad = DataSource::air_data(7, 0x0E, 41000, 250);
    bad.garble = true;
    set.subsystems.push(Box::new(bad));
    set.run(300);
    let iop = set.gpcs[0].iop.borrow();
    assert!(iop.bces[7].error, "garbled sensor rejected");
    assert_ne!(iop.bces[7].status & (0b111 << 20), 0, "SEV recorded");
}

#[test]
fn mass_memory_loads_a_program_block() {
    // The tape units the crew loaded software from between flight
    // phases. Block contents are this project's, not NASA's.
    let mut set = RedundantSet::new(vec![poller(7, 4)]);
    set.subsystems.push(Box::new(
        MassMemory::new(7, 0x0E)
            .with_block("OPS 1 ASCENT", vec![0x1111, 0x2222, 0x3333, 0x4444])
            .with_block("OPS 3 ENTRY", vec![0xAAAA, 0xBBBB, 0xCCCC, 0xDDDD]),
    ));
    set.run(300);
    let g = &set.gpcs[0];
    let got: Vec<u16> = (0..4).map(|i| g.cpu.mem.read_h(BUF + i).unwrap()).collect();
    assert_eq!(got, vec![0x1111, 0x2222, 0x3333, 0x4444], "first block loaded");
    let mm = set.subsystems[0]
        .as_any()
        .downcast_ref::<MassMemory>()
        .unwrap();
    assert_eq!(mm.loads, 1);
    assert_eq!(mm.blocks[mm.selected].0, "OPS 1 ASCENT");
}
