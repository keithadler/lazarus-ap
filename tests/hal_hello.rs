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
    // Trap addresses auto-resolved from the symbols JSON the way yaGPC2
    // does (IOINIT section base -> OUTRAP/CNTRAP; INTRAP/IOCODE/IOBUF
    // symbols).
    let mut ucp = HalUcp::from_symbols_json(&json).expect("symbols resolve");
    assert_eq!(ucp.outrap, 65806 + 0x11);
    assert_eq!(ucp.iocode_addr, 398);
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

/// Parity against yaGPC2's actual output (goldens captured from a
/// locally-built yaGPC2 running the same fixtures). Two known gaps,
/// tracked in docs/ROADMAP.md:
/// 1. emit_field column/pagination layout is not yet ported (our output
///    joins fields with single spaces);
/// 2. 176-P's ACCEL component 0 (golden 9.9999964E-02) comes out as a
///    characteristic-zero garbage value — components 1 and 2 match the
///    golden EXACTLY, so this is an arithmetic bug in one of our §8
///    float operations, not an output bug. Hunt: watch stores to the
///    ACCEL vector and diff the computation against yaGPC2.
/// Byte-for-byte parity with the reference emulator (goldens captured
/// from a locally-built yaGPC2 on the same fixtures).
#[test]
fn golden_parity() {
    for (fcm_path, json_path, golden_path) in [
        (
            "roms/hello/hello.fcm",
            "roms/hello/hello-lnk101.json",
            "roms/hello/golden.txt",
        ),
        (
            "roms/p176/176-P.fcm",
            "roms/p176/176-P-lnk101.json",
            "roms/p176/golden.txt",
        ),
        // EOF mid-READ caught by an ON ERROR handler (SCAL-frame unwind)
        (
            "roms/read_eof/read_eof_onerror.fcm",
            "roms/read_eof/read_eof_onerror-lnk101.json",
            "roms/read_eof/golden.txt",
        ),
    ] {
        let bytes = std::fs::read(fcm_path).unwrap();
        let json = std::fs::read_to_string(json_path).unwrap();
        let golden = std::fs::read_to_string(golden_path).unwrap();
        let mut cpu = Cpu::new(Memory::full());
        fcm::boot(&mut cpu, &bytes, Some(&json)).unwrap();
        let mut ucp = HalUcp::from_symbols_json(&json).unwrap();
        assert_eq!(run_hal(&mut cpu, &mut ucp, 3_000_000), HalRun::Done);
        assert_eq!(ucp.output, golden, "parity: {fcm_path}");
    }
}

/// READ support: the read_write fixture (real HALSFC program) consumes
/// "42, 3.14" through INTRAP and echoes both values. Golden from the
/// locally-built yaGPC2 with the same stdin.
#[test]
fn read_write_parity() {
    let bytes = std::fs::read("roms/read_write/read_write.fcm").unwrap();
    let json = std::fs::read_to_string("roms/read_write/read_write-lnk101.json").unwrap();
    let golden = std::fs::read_to_string("roms/read_write/golden.txt").unwrap();
    let mut cpu = Cpu::new(Memory::full());
    fcm::boot(&mut cpu, &bytes, Some(&json)).unwrap();
    let mut ucp = HalUcp::from_symbols_json(&json).unwrap();
    ucp.input = "42, 3.14\n".to_string();
    assert_eq!(run_hal(&mut cpu, &mut ucp, 3_000_000), HalRun::Done);
    assert_eq!(ucp.output, golden);
}

/// The closed loop: LAZARUS.hal was written for this project, compiled
/// by the real HAL/S-FC (release 32V0, all seven passes built from the
/// restored XPL source), linked by our tools/lnk_lite.py against the
/// hello fixture's runtime layout, and runs here.
#[test]
fn our_own_hal_s_program_runs() {
    let bytes = std::fs::read("roms/lazarus/LAZARUS.fcm").unwrap();
    let json = std::fs::read_to_string("roms/lazarus/LAZARUS-lnk101.json").unwrap();
    let mut cpu = Cpu::new(Memory::full());
    fcm::boot(&mut cpu, &bytes, Some(&json)).unwrap();
    let mut ucp = HalUcp::from_symbols_json(&json).unwrap();
    assert_eq!(run_hal(&mut cpu, &mut ucp, 3_000_000), HalRun::Done);
    assert!(ucp.output.starts_with("LAZARUS AP LIVES\n"));
    assert!(ucp.output.ends_with("SET IS GO\n"));
    assert_eq!(ucp.output.matches("GPC VOTE              42").count(), 3);
}

/// The assembly path: LAZASM.asm was assembled by the genuine ASM101S
/// assembler and linked standalone by tools/lnk_lite.py. It computes a
/// value, copies a witness pattern, and exits via the SVC convention.
#[test]
fn asm101s_assembled_program_runs() {
    let bytes = std::fs::read("roms/lazasm/LAZASM.fcm").unwrap();
    let json = std::fs::read_to_string("roms/lazasm/LAZASM-lnk101.json").unwrap();
    let mut cpu = Cpu::new(Memory::full());
    fcm::boot(&mut cpu, &bytes, Some(&json)).unwrap();
    let mut ucp = HalUcp::new(u32::MAX >> 1, 0, 0, 0); // no runtime I/O
    let r = run_hal(&mut cpu, &mut ucp, 10_000);
    println!("r={r:?} ic={:04X}", cpu.psw.ic);
    // CSECT at 0x100: DONE +0xB, ANSWER +0xC, WITNESS +0xD (ASM101S
    // symbol table).
    println!("answer={:04X} witness={:04X}",
        cpu.mem.read_h(0x10C).unwrap(), cpu.mem.read_h(0x10D).unwrap());
    assert_eq!(r, HalRun::Done, "ends via SVC 0x15 pointer");
    assert_eq!(cpu.mem.read_h(0x10C).unwrap(), 42, "computed 7+35");
    assert_eq!(cpu.mem.read_h(0x10D).unwrap(), 0x5A5A, "witness copied");
}

/// Genuine NASA flight code: RUNASM/SQRT.asm — the Shuttle runtime's
/// single-precision square root (hyperbolic seed + two Newton-Raphson
/// passes) — assembled by the real ASM101S, called by a driver via
/// BAL 4 with the AEXIT frame convention, computing sqrt(2.0).
#[test]
#[ignore = "flight SQRT runs its full algorithm and returns; R1-based constant fetch lands wrong (SRS scaling vs ASM101S emission — see roms/nasa/CENSUS.md)"]
fn nasa_flight_sqrt_computes_sqrt2() {
    let bytes = std::fs::read("/tmp/SQRTRUN.fcm").or_else(|_| std::fs::read("roms/nasa/SQRTRUN.fcm")).unwrap();
    let mut cpu = Cpu::new(Memory::full());
    fcm::boot(&mut cpu, &bytes, Some(r#"{"entryPoint": 256}"#)).unwrap();
    let mut ucp = HalUcp::new(u32::MAX >> 1, 0, 0, 0);
    let r = run_hal(&mut cpu, &mut ucp, 100_000);
    let result = cpu.mem.read_f(0x10D).unwrap();
    println!("r={r:?} ic={:04X} RESULT={result:08X}", cpu.psw.ic);
    let expect = 0x4116A09Eu32; // sqrt(2) in IBM hex float
    assert!(result.abs_diff(expect) <= 2, "sqrt(2): {result:08X} vs {expect:08X}");
    assert_eq!(r, HalRun::Done);
}
