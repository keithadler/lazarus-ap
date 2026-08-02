//! Golden-trace support: run a program, producing a deterministic textual
//! trace of architectural state after every step, suitable for committing
//! as an expected-output file and diffing in CI.

use crate::cpu::{Cpu, Halt};

/// One line per executed step: step number, IC (16-bit, as fetched-from
/// after expansion it may differ — the raw PSW field is shown), CC, carry,
/// overflow, and the current register set. A final line reports the halt
/// reason.
pub fn trace_run(cpu: &mut Cpu, max_steps: u64) -> (String, Halt) {
    let mut out = String::new();
    let halt = loop {
        if cpu.steps >= max_steps {
            break Halt::StepLimit;
        }
        if cpu.psw.wait {
            break Halt::Wait;
        }
        let before = cpu.psw.ic;
        match cpu.step() {
            Ok(at) => {
                out.push_str(&state_line(cpu, at));
                if cpu.psw.ic == before {
                    break Halt::SelfLoop { at: before };
                }
            }
            Err(t) => break Halt::Trap(t),
        }
    };
    out.push_str(&format!("halt: {}\n", halt_str(&halt)));
    (out, halt)
}

fn state_line(cpu: &Cpu, at: u32) -> String {
    let p = &cpu.psw;
    let mut s = format!(
        "{:04} at={:05X} ic={:04X} cc={:02b} c={} v={} ",
        cpu.steps,
        at,
        p.ic,
        p.cc,
        p.carry as u8,
        p.overflow as u8
    );
    for n in 0..8 {
        s.push_str(&format!("r{}={:08X} ", n, cpu.r(n)));
    }
    s.pop();
    s.push('\n');
    s
}

fn halt_str(h: &Halt) -> String {
    match h {
        Halt::SelfLoop { at } => format!("self-loop at {at:04X}"),
        Halt::Wait => "wait state".into(),
        Halt::StepLimit => "step limit".into(),
        Halt::Trap(t) => format!("trap: {t:?}"),
    }
}

/// Dump of final register/PSW state plus a window of memory, for tests
/// that assert on end state rather than per-step traces.
pub fn dump_state(cpu: &Cpu, mem_from: u32, mem_halfwords: u32) -> String {
    let p = &cpu.psw;
    let mut s = format!(
        "ic={:04X} cc={:02b} c={} v={}\n",
        p.ic, p.cc, p.carry as u8, p.overflow as u8
    );
    for n in 0..8 {
        s.push_str(&format!("r{}={:08X}\n", n, cpu.r(n)));
    }
    let mut addr = mem_from;
    while addr < mem_from + mem_halfwords {
        s.push_str(&format!("m[{addr:05X}]="));
        s.push_str(&format!("{:04X}\n", cpu.mem.read_h(addr).unwrap_or(0)));
        addr += 1;
    }
    s
}
