//! Execution-time budget for a program run.
//!
//! Adds up the HAL/S-FC compiler's own per-instruction times (see
//! src/timing.rs for provenance) over a whole run, and reports how much
//! of a 40 Hz guidance frame it would have consumed.
//!
//! The unit is UNVERIFIED — HAL/S-FC printed bare numbers. Microseconds
//! is the conventional assumption and is what `--assume-microseconds`
//! applies; without that flag the output stays in raw time units.
//!
//! Usage: lazap-time <image.fcm> [--steps N] [--assume-microseconds]

use lazarus_ap::halucp::{run_hal_traced, HalUcp};
use lazarus_ap::timing::{instr_time, pre_n, Budget};
use lazarus_ap::{decode, fcm, Cpu, Memory};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut file = None;
    let mut steps = 3_000_000usize;
    let mut assume_us = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--steps" => steps = it.next().and_then(|s| s.parse().ok()).unwrap_or(steps),
            "--assume-microseconds" => assume_us = true,
            _ => file = Some(a.clone()),
        }
    }
    let file = file.expect("usage: lazap-time <image.fcm> [--steps N]");
    let bytes = std::fs::read(&file).expect("read fcm");
    let stem = file.strip_suffix(".fcm").unwrap_or(&file);
    let json = std::fs::read_to_string(format!("{stem}-lnk101.json")).ok();

    let mut cpu = Cpu::new(Memory::full());
    fcm::boot(&mut cpu, &bytes, json.as_deref()).unwrap();
    let mut ucp = json
        .as_deref()
        .and_then(HalUcp::from_symbols_json)
        .unwrap_or_else(|| HalUcp::new(u32::MAX >> 1, 0, 0, 0));

    let mut budget = Budget::default();
    // Each instruction's time can depend on whether its branch was
    // taken, which is only known once the next address is in hand. So
    // every observation settles the *previous* instruction.
    let mut pending: Option<(decode::Decoded, u32, u32)> = None;
    let mut worst: (f64, String) = (0.0, String::new());

    let settle = |budget: &mut Budget,
                  worst: &mut (f64, String),
                  p: &Option<(decode::Decoded, u32, u32)>,
                  next_ia: u32| {
        if let Some((dec, n, seq)) = p {
            let t = instr_time(dec, *n, next_ia != *seq);
            if let Some(v) = t {
                if v > worst.0 {
                    *worst = (v, dec.instr.mnemonic().to_string());
                }
            }
            budget.add(t);
        }
    };

    let r = run_hal_traced(&mut cpu, &mut ucp, steps, |cpu, _ucp, nia| {
        settle(&mut budget, &mut worst, &pending, nia);
        let hw1 = cpu.mem.read_h(nia).unwrap_or(0);
        let hw2 = cpu.mem.read_h(nia + 1).unwrap_or(0);
        pending = decode::decode(hw1, hw2).ok().map(|d| {
            let n = pre_n(&d, cpu.r(d.r1));
            (d, n, nia + d.len as u32)
        });
    });
    // The last instruction never gets a following observation; settle it
    // against the final IC.
    settle(&mut budget, &mut worst, &pending, cpu.expand_branch(cpu.psw.ic));

    let total = budget.timed + budget.untimed;
    println!("{file}: {r:?}");
    println!("  instructions   {total}  ({} timed, {} untimed)", budget.timed, budget.untimed);
    println!("  execution time {:.2} time units", budget.units);
    if total > 0 {
        println!("  mean           {:.3} units/instruction", budget.units / total as f64);
    }
    if !worst.1.is_empty() {
        println!("  slowest single {} at {:.3} units", worst.1, worst.0);
    }
    if assume_us {
        // ONLY under the unconfirmed microsecond assumption.
        let ms = budget.units / 1000.0;
        println!("  --- assuming 1 unit = 1 microsecond (UNVERIFIED) ---");
        println!("  wall time      {:.3} ms", ms);
        println!("  40 Hz frames   {:.3}  (one frame = 25 ms)", budget.frames_at_40hz());
    } else {
        println!("  (pass --assume-microseconds for a frame-budget reading;");
        println!("   the unit is not confirmed — see src/timing.rs)");
    }
}
