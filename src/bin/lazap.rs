//! Command-line runner: assemble an AP-101S program, execute it, print a
//! per-step trace.
//!
//! Usage: lazap <program.asm> [--steps N] [--quiet]

use lazarus_ap::{asm, trace, Cpu, Memory};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut file = None;
    let mut steps: u64 = 10_000;
    let mut quiet = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--steps" => {
                steps = it
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_else(|| die("--steps needs a number"));
            }
            "--quiet" => quiet = true,
            _ if file.is_none() => file = Some(a.clone()),
            _ => die("unexpected argument"),
        }
    }
    let file = file.unwrap_or_else(|| die("usage: lazap <program.asm> [--steps N] [--quiet]"));
    let src = std::fs::read_to_string(&file)
        .unwrap_or_else(|e| die(&format!("cannot read {file}: {e}")));
    let prog = asm::assemble(&src).unwrap_or_else(|e| die(&format!("{file}: {e}")));

    let mut cpu = Cpu::new(Memory::full());
    prog.load(&mut cpu.mem).unwrap_or_else(|e| die(&format!("load: {e:?}")));
    cpu.psw.ic = prog.entry;

    let (trace_text, halt) = trace::trace_run(&mut cpu, steps);
    if quiet {
        println!("halted: {halt:?} after {} steps", cpu.steps);
    } else {
        print!("{trace_text}");
    }
}

fn die(msg: &str) -> ! {
    eprintln!("lazap: {msg}");
    std::process::exit(1);
}
