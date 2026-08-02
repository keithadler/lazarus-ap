//! Command-line runner: assemble an AP-101S program, execute it, print a
//! per-step trace — or run a linked `.fcm` memory image from the
//! HAL/S toolchain (HALSFC -> lnk101), auto-detecting `<stem>-lnk101.json`
//! or `<stem>.sym.json` for the entry point and runtime-I/O traps.
//!
//! Usage: lazap <program.asm | image.fcm> [--steps N] [--quiet]

use lazarus_ap::halucp::{run_hal, HalRun, HalUcp};
use lazarus_ap::{asm, fcm, trace, Cpu, Memory};

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
    if file.to_ascii_lowercase().ends_with(".fcm") {
        run_fcm(&file, steps);
        return;
    }
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

/// Run a HAL/S toolchain image: load, resolve symbols, trap runtime
/// I/O, print the program's WRITE output.
fn run_fcm(file: &str, steps: u64) {
    let bytes =
        std::fs::read(file).unwrap_or_else(|e| die(&format!("cannot read {file}: {e}")));
    let stem = file.strip_suffix(".fcm").unwrap_or(file);
    let json = std::fs::read_to_string(format!("{stem}-lnk101.json"))
        .or_else(|_| std::fs::read_to_string(format!("{stem}.sym.json")))
        .ok();
    let mut cpu = Cpu::new(Memory::full());
    fcm::boot(&mut cpu, &bytes, json.as_deref()).unwrap_or_else(|e| {
        die(&format!("image load failed: {e:?}"))
    });
    let mut ucp = match json.as_deref().and_then(HalUcp::from_symbols_json) {
        Some(u) => u,
        None => {
            eprintln!("warning: no symbols JSON found; running without runtime I/O traps");
            HalUcp::new(u32::MAX >> 1, 0, 0, 0)
        }
    };
    let r = run_hal(&mut cpu, &mut ucp, steps as usize);
    print!("{}", ucp.output);
    if !ucp.output.ends_with('\n') {
        println!();
    }
    match r {
        HalRun::Done => {}
        other => eprintln!("run ended: {other:?} (ic={:04X})", cpu.psw.ic),
    }
}

fn die(msg: &str) -> ! {
    eprintln!("lazap: {msg}");
    std::process::exit(1);
}
