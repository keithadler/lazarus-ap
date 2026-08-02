//! Trace generator: run an .fcm and emit a JSON execution trace for the
//! graphical walkthrough (docs/walkthrough.html) — per step: the 19-bit
//! instruction address, mnemonic, registers, condition code, and how
//! much program output existed. Output events land in the same file.
//!
//! Usage: lazap-trace <image.fcm> [--steps N] [--max-trace M] > trace.json

use lazarus_ap::halucp::{run_hal_traced, HalUcp};
use lazarus_ap::{decode, fcm, Cpu, Memory};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut file = None;
    let mut steps = 3_000_000usize;
    let mut max_trace = 4000usize;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--steps" => steps = it.next().and_then(|s| s.parse().ok()).unwrap_or(steps),
            "--max-trace" => {
                max_trace = it.next().and_then(|s| s.parse().ok()).unwrap_or(max_trace)
            }
            _ => file = Some(a.clone()),
        }
    }
    let file = file.expect("usage: lazap-trace <image.fcm>");
    let bytes = std::fs::read(&file).expect("read fcm");
    let stem = file.strip_suffix(".fcm").unwrap_or(&file);
    let json = std::fs::read_to_string(format!("{stem}-lnk101.json")).ok();
    let mut cpu = Cpu::new(Memory::full());
    fcm::boot(&mut cpu, &bytes, json.as_deref()).unwrap();
    let mut ucp = json
        .as_deref()
        .and_then(HalUcp::from_symbols_json)
        .unwrap_or_else(|| HalUcp::new(u32::MAX >> 1, 0, 0, 0));

    let mut rows = String::new();
    let mut n = 0usize;
    let r = run_hal_traced(&mut cpu, &mut ucp, steps, |cpu, ucp, nia| {
        if n >= max_trace {
            return;
        }
        let hw1 = cpu.mem.read_h(nia).unwrap_or(0);
        let hw2 = cpu.mem.read_h(nia + 1).unwrap_or(0);
        let m = decode::decode(hw1, hw2)
            .map(|d| d.instr.mnemonic())
            .unwrap_or("???");
        if n > 0 {
            rows.push(',');
        }
        rows.push_str(&format!(
            "[{},\"{}\",{},[{}],{},{}]",
            nia,
            m,
            hw1,
            (0..8)
                .map(|i| format!("\"{:08X}\"", cpu.r(i)))
                .collect::<Vec<_>>()
                .join(","),
            cpu.psw.cc,
            ucp.output.len(),
        ));
        n += 1;
    });
    let name = std::path::Path::new(&file)
        .file_name()
        .unwrap()
        .to_string_lossy();
    println!(
        "{{\"program\":\"{}\",\"result\":\"{:?}\",\"traced\":{},\"steps\":[{}],\"output\":{}}}",
        name,
        r,
        n,
        rows,
        serde_json_string(&ucp.output)
    );
}

/// Minimal JSON string escaping (no serde dependency).
fn serde_json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
