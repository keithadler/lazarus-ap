//! THE test: a genuine HAL/S program — compiled by the recovered HALSFC
//! compiler, linked by lnk101 (artifact from the Virtual AGC project's
//! yaGPC2 fixtures) — runs on Lazarus AP: FCM loader, AP-101S CPU, and
//! the halucp runtime-I/O trap layer, end to end.

use lazarus_ap::halucp::{run_hal, HalRun, HalUcp};
use lazarus_ap::{fcm, Cpu, Memory};

#[test]
fn real_halsfc_hello_world_runs() {
    let bytes = std::fs::read("roms/hello/hello.fcm").expect("fixture present");
    let json = std::fs::read_to_string("roms/hello/hello-lnk101.json").unwrap();
    let mut cpu = Cpu::new(Memory::full());
    fcm::boot(&mut cpu, &bytes, Some(&json)).unwrap();
    // Trap addresses resolved from the symbols JSON the way yaGPC2 does:
    // IOINIT section base 65806 (OUTRAP +0x11, CNTRAP +0x40), INTRAP
    // 65858, IOCODE 398, IOBUF 400.
    let mut ucp = HalUcp::new(65806, 65858, 398, 400);
    let r = run_hal(&mut cpu, &mut ucp, 2_000_000);
    println!("--- captured output ---\n{}\n---", ucp.output);
    println!("result: {r:?}, ic={:04X} bsr={}", cpu.psw.ic, cpu.psw.bsr);
    assert_eq!(r, HalRun::Done, "program ends via SVC 0x0015");
    for needle in [
        "THE BEGINNING",
        "HELLO, WORLD!",
        "RON BURKEY SAYS ISN'T THIS FUN?",
        "THE END",
    ] {
        assert!(ucp.output.contains(needle), "missing: {needle}");
    }
    assert_eq!(ucp.output.matches("HELLO, WORLD!").count(), 5);
    assert_eq!(ucp.output.matches("ISN'T THIS FUN?").count(), 20);
}
