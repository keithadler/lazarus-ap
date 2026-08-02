//! Run a linked flight-routine image and print the result word at
//! DRIVER+0xE as hex and as a decoded IBM hexfloat.
//!
//! Usage: lazap-call <image.fcm> [--steps N] [--dp]

use lazarus_ap::halucp::{run_hal, HalRun, HalUcp};
use lazarus_ap::{fcm, Cpu, Memory};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let file = a.iter().find(|s| !s.starts_with("--")).expect("image.fcm");
    let dp = a.iter().any(|s| s == "--dp");
    let steps = a
        .windows(2)
        .find(|w| w[0] == "--steps")
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(400_000usize);
    let bytes = std::fs::read(file).expect("read");
    let mut cpu = Cpu::new(Memory::full());
    fcm::boot(&mut cpu, &bytes, Some(r#"{"entryPoint": 256}"#)).unwrap();
    let mut ucp = HalUcp::new(u32::MAX >> 1, 0, 0, 0);
    let r = run_hal(&mut cpu, &mut ucp, steps);
    let hi = cpu.mem.read_f(0x10E).unwrap();
    let u = if dp {
        lazarus_ap::float::unpack_long(hi, cpu.mem.read_f(0x110).unwrap())
    } else {
        lazarus_ap::float::unpack_short(hi)
    };
    let v = if u.is_zero() {
        0.0
    } else {
        let m = u.frac as f64 * (16f64).powi(u.ch - 78);
        if u.neg { -m } else { m }
    };
    println!("{:?} {:08X} {}", r, hi, v);
    if r != HalRun::Done {
        std::process::exit(2);
    }
}
