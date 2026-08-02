//! Golden-trace integration tests: assemble a program, run it end to end,
//! and compare the full architectural trace against a committed expected
//! trace (tests/golden/*.txt).
//!
//! Regenerate a golden file by running the test with
//! `UPDATE_GOLDEN=1 cargo test --test golden` and reviewing the diff.

mod common;

use lazarus_ap::{asm, trace, Cpu, Memory};

/// Sum an array of fullwords, count down a tally, and exercise shifts and
/// logical ops; results stored to memory. Halts with a self-loop.
const SUM_PROGRAM: &str = "
        ORG 0x40
ARR:    DC F(11),F(22),F(33),F(44),F(55)
SUMOUT: DC F(0)
TALLY:  DC H(3)

        ORG 0x100
START:  LFXI 5,0        ; running sum
        LA   1,ARR      ; array cursor (bits 0-15)
        LFXI 2,5        ; element count in bits 0-15 of R2
LOOP:   L    3,0(1)     ; load element via base R1
        AR   5,3        ; sum += element
        LA   1,2(1)     ; cursor += 2 halfwords (§4.15 note: R1=B2
                        ;   increments R1 by the displacement)
        BCT  2,LOOP
        ST   5,SUMOUT   ; 165
TDLOOP: TD   TALLY
        BC   1,TDLOOP   ; tally down 3,2,1 -> 0 (CC=01 while positive)
        LFXI 4,1
        SLL  4,10       ; 0x0400 in the low half... (1<<10)
        SRR  4,16       ; rotate: 0x0400 -> upper half
        XUL  4,4        ; swap halves back
        DONE: B DONE
";

fn run_golden(name: &str, src: &str, max_steps: u64) {
    let prog = asm::assemble(src).unwrap_or_else(|e| panic!("asm: {e}"));
    let mut cpu = Cpu::new(Memory::new(0x2000));
    prog.load(&mut cpu.mem).unwrap();
    cpu.psw.ic = prog.entry;
    let (text, _halt) = trace::trace_run(&mut cpu, max_steps);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.txt"));
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &text).unwrap();
        eprintln!("updated {}", path.display());
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("missing golden file {}: {e}", path.display()));
    assert_eq!(
        text, expected,
        "trace mismatch for {name}; run with UPDATE_GOLDEN=1 and review the diff"
    );
}

#[test]
fn golden_sum_program() {
    run_golden("sum_program", SUM_PROGRAM, 200);
}

#[test]
fn sum_program_end_state() {
    // Independent of the trace file: the program's architectural outcome.
    let prog = asm::assemble(SUM_PROGRAM).unwrap();
    let mut cpu = Cpu::new(Memory::new(0x2000));
    prog.load(&mut cpu.mem).unwrap();
    cpu.psw.ic = prog.entry;
    let halt = cpu.run(200);
    assert!(matches!(halt, lazarus_ap::Halt::SelfLoop { .. }));
    // 11+22+33+44+55 = 165
    assert_eq!(cpu.mem.read_f(0x4A).unwrap(), 165);
    // tally counted down to zero
    assert_eq!(cpu.mem.read_h(0x4C).unwrap(), 0);
    // LFXI 4,1 -> 0x00010000; SLL 10 -> 0x04000000; SRR 16 -> 0x00000400;
    // XUL 4,4 swaps halves -> 0x04000000.
    assert_eq!(cpu.r(4), 0x0400_0000);
}
