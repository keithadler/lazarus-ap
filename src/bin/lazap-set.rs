//! Watch the redundant set vote: five GPCs (four PASS + one BFS-style
//! listener) run one identical software image; GPC 2's sensor is
//! corrupted; the healthy machines catch it over the flight-critical
//! buses and vote it out, while the actuator force-votes the bad
//! command into irrelevance.
//!
//! Usage: lazap-set [--fault N] [--kill N] [--fast]
//!   --fault N   corrupt GPC N's sensor input (default 2)
//!   --kill N    kill GPC N outright instead (fail-silent scenario)
//!   --fast      no animation delays

use lazarus_ap::asm::assemble;
use lazarus_ap::demo::{build_gpc, cpu_program, MBOX};
use lazarus_ap::gpc::{ForceVotedActuator, RedundantSet};
use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut fault: Option<u8> = Some(2);
    let mut kill: Option<usize> = None;
    let mut fast = false;
    let mut trace = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--fault" => fault = it.next().and_then(|s| s.parse().ok()),
            "--kill" => {
                kill = it.next().and_then(|s| s.parse().ok());
                fault = None;
            }
            "--fast" => fast = true,
            "--trace" => trace = true,
            _ => {}
        }
    }

    let verdict_addr = assemble(&cpu_program(0)).unwrap().label("VERDICT").unwrap();
    let mut gpcs: Vec<_> = (0..4)
        .map(|id| build_gpc(id, Some(id) == fault))
        .collect();
    let bfs = build_gpc(4, false);
    bfs.iop.borrow_mut().mia_xmtr_enable = 0; // pure listener
    gpcs.push(bfs);
    if let Some(k) = kill {
        // illegal encoding at the entry: the machine dies at once
        gpcs[k].cpu.mem.write_h(0x100, 0b00110_001_11100_010).unwrap();
    }
    let mut set = RedundantSet::new(gpcs);
    set.actuators.push(ForceVotedActuator::new(8, 5));

    if trace {
        // JSON trace for the graphical walkthrough: sampled set state.
        let mut rows = String::new();
        let mut nrows = 0;
        for tick in 0..1600 {
            set.step();
            if tick % 4 != 0 {
                continue;
            }
            let mut gs = String::new();
            for (i, g) in set.gpcs.iter().enumerate() {
                if i > 0 {
                    gs.push(',');
                }
                let boxes: Vec<String> = (0..4)
                    .map(|j| g.cpu.mem.read_h(MBOX + j).unwrap().to_string())
                    .collect();
                let v = g.cpu.mem.read_h(verdict_addr).unwrap();
                let dead = set.dead[i].is_some();
                gs.push_str(&format!(
                    "[[{}],{},{}]",
                    boxes.join(","),
                    v,
                    if dead { 1 } else { 0 }
                ));
            }
            let out = set.actuators[0].output().unwrap_or(-1);
            let byp: Vec<String> = (0..4)
                .filter(|&p| set.actuators[0].bypassed[p])
                .map(|p| p.to_string())
                .collect();
            if nrows > 0 {
                rows.push(',');
            }
            rows.push_str(&format!("[{tick},[{gs}],{out},[{}]]", byp.join(",")));
            nrows += 1;
        }
        println!("{{\"rows\":[{rows}]}}");
        return;
    }

    println!("AP-101S REDUNDANT SET: 4 PASS + 1 BFS LISTENER");
    match (fault, kill) {
        (Some(f), _) => println!("fault injected: GPC {f} sensor corrupted\n"),
        (_, Some(k)) => println!("fault injected: GPC {k} killed (fail-silent)\n"),
        _ => println!("no fault injected\n"),
    }

    for round in 0..40 {
        set.run(100);
        let mut out = String::new();
        out.push_str("GPC | ROLE     | MAILBOXES (heard on buses) | VERDICT | STATE\n");
        out.push_str("----+----------+----------------------------+---------+------\n");
        for (i, g) in set.gpcs.iter().enumerate() {
            let role = if i == 4 { "BFS shadow" } else { "PASS" };
            let boxes: Vec<String> = (0..4)
                .map(|j| {
                    let v = g.cpu.mem.read_h(MBOX + j).unwrap();
                    if v == 0 { "  --".into() } else { format!("{v:4}") }
                })
                .collect();
            let verdict = g.cpu.mem.read_h(verdict_addr).unwrap();
            let vs = if verdict == 0 && set.dead[i].is_none() {
                "  .....".to_string()
            } else {
                format!(
                    " {}",
                    (0..4)
                        .map(|b| if verdict & (1 << b) != 0 { 'X' } else { '.' })
                        .collect::<String>()
                )
            };
            let state = if set.dead[i].is_some() {
                "DEAD"
            } else if verdict != 0 {
                "VOTED"
            } else {
                "run"
            };
            out.push_str(&format!(
                "  {i} | {role:<8} | {}                | {vs:<7} | {state}\n",
                boxes.join(" ")
            ));
        }
        if let Some(v) = set.actuators[0].output() {
            let byp: Vec<String> = (0..4)
                .filter(|&p| set.actuators[0].bypassed[p])
                .map(|p| p.to_string())
                .collect();
            out.push_str(&format!(
                "\nACTUATOR force-vote: commands {:?} -> output {v}{}\n",
                set.actuators[0].ports,
                if byp.is_empty() {
                    String::new()
                } else {
                    format!("  (port {} BYPASSED)", byp.join(","))
                }
            ));
        }
        print!("\x1b[2J\x1b[H{out}");
        std::io::stdout().flush().ok();

        // settled? every live PASS GPC has a verdict
        let settled = (0..4).all(|i| {
            set.dead[i].is_some() || set.gpcs[i].cpu.mem.read_h(verdict_addr).unwrap() != 0
        });
        if settled && round > 2 {
            break;
        }
        if !fast {
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
    }

    // the verdict tally
    println!("\n=== VOTE TALLY ===");
    let mut accused = [0u32; 4];
    for i in 0..5 {
        if set.dead[i].is_some() {
            println!("GPC {i}: silent (dead)");
            continue;
        }
        let v = set.gpcs[i].cpu.mem.read_h(verdict_addr).unwrap();
        let names: Vec<String> = (0..4)
            .filter(|b| v & (1 << b) != 0)
            .map(|b| format!("GPC {b}"))
            .collect();
        for b in 0..4 {
            if v & (1 << b) != 0 {
                accused[b] += 1;
            }
        }
        println!(
            "GPC {i} votes against: {}",
            if names.is_empty() { "nobody".into() } else { names.join(", ") }
        );
    }
    if let Some((worst, n)) = accused.iter().enumerate().max_by_key(|&(_, n)| n) {
        if *n >= 2 {
            println!("\n>>> GPC {worst} VOTED OUT ({n} votes against) <<<");
        } else {
            println!("\nno majority against any machine");
        }
    }
}
