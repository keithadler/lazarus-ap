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

/// The closed loop: a simulated sensor reports a direction over the
/// bus, and genuine NASA flight routines turn it into a pointing
/// angle. The vector is invented; VV10S3 (UNIT VECTOR), VV0SN, SQRT
/// and VV6S3 (dot product) are the Shuttle's own code.
#[test]
fn flight_math_on_simulated_sensor_data() {
    use lazarus_ap::halucp::{run_hal, HalRun, HalUcp};
    use lazarus_ap::{fcm, Cpu, Memory};

    // The sensor reports a direction 45 degrees off the reference axis:
    // (3, 3, 0) as IBM hexfloat halfwords.
    let f = |v: f64| -> [u16; 2] {
        if v == 0.0 { return [0, 0]; }
        let (mut av, mut ch) = (v.abs(), 64i32);
        while av >= 1.0 { av /= 16.0; ch += 1; }
        while av < 1.0 / 16.0 { av *= 16.0; ch -= 1; }
        let w = (((v < 0.0) as u32) << 31) | ((ch as u32 & 0x7F) << 24)
            | ((av * (1u32 << 24) as f64) as u32 & 0xFF_FFFF);
        [(w >> 16) as u16, w as u16]
    };
    let mut words = Vec::new();
    for v in [3.0, 3.0, 0.0] {
        words.extend_from_slice(&f(v));
    }

    // Boot the pipeline image, then let a BCE poll the sensor straight
    // into the driver's own input vector (SENSM1 + one fullword).
    let bytes = std::fs::read("roms/nasa/ATTRUN.fcm").unwrap();
    let mut cpu = Cpu::new(Memory::full());
    fcm::boot(&mut cpu, &bytes, Some(r#"{"entryPoint": 256}"#)).unwrap();
    const SENSOR_VEC: u32 = 0x118; // SENSM1 is 0x116; the vector follows
    let poll: u32 = (0x0E << 19) | (1 << 16) | 6;
    cpu.mem
        .load_halfwords(
            0x1000,
            &[
                0xF200, SENSOR_VEC as u16, 0xB000 | 300, 0xC000,
                0b11110001_00000000, 5, (poll >> 16) as u16, poll as u16,
                0xE800, 0x0800,
            ],
        )
        .unwrap();
    let mut gpc = Gpc { cpu, iop: Default::default() };
    {
        let mut iop = gpc.iop.borrow_mut();
        iop.halted = false;
        iop.bces[7].busy = true;
        iop.bces[7].pc = 0x1000;
        iop.mia_xmtr_enable = 1 << (31 - 7u32);
        iop.mia_rcvr_enable = 1 << (31 - 7u32);
    }
    gpc.cpu.psw.wait = true; // hold the CPU while the sensor is read
    let mut set = RedundantSet::new(vec![gpc]);
    set.subsystems
        .push(Box::new(DataSource::new(7, 0x0E, words)));
    set.run(400);

    // The reading landed; now run the flight mathematics on it.
    let mut cpu = std::mem::replace(&mut set.gpcs[0].cpu, Cpu::new(Memory::new(4)));
    assert_ne!(cpu.mem.read_h(SENSOR_VEC).unwrap(), 0, "sensor data arrived");
    cpu.psw.wait = false;
    cpu.psw.ic = 0x100;
    let mut ucp = HalUcp::new(u32::MAX >> 1, 0, 0, 0);
    assert_eq!(run_hal(&mut cpu, &mut ucp, 400_000), HalRun::Done);

    // cos(45 degrees) = 0.7071 between (3,3,0) and the +X reference.
    let w = cpu.mem.read_f(0x130).unwrap();
    let u = lazarus_ap::float::unpack_short(w);
    let cos = u.frac as f64 * (16f64).powi(u.ch - 78) * if u.neg { -1.0 } else { 1.0 };
    println!("pointing cosine = {cos} ({:08X})", w);
    assert!((cos - 0.70710678).abs() < 1e-5, "45 degrees off axis: {cos}");
}
