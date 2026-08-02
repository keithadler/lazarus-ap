# Lazarus AP

A faithful, test-driven emulator of the IBM AP-101 — the general-purpose
computer (GPC) that flew as the Space Shuttle's flight computer.

**Phase 1** (this repository's current state): a CPU + main-storage
emulator for the **AP-101S** "Shuttle instruction set", built strictly
from primary sources — chiefly IBM 85-C67-001, *Space Shuttle Model
AP-101S Principles of Operation with Shuttle Instruction Set* — with an
instruction-level test suite. No HAL/S compiler, no flight software yet.

## What works

- 121 instructions: the fixed-point, branching, shift, and logical
  sections plus (phase 2) the full floating-point set — IBM hexadecimal
  short/long formats with guard-digit prealignment and the §8.8
  exception rules — and the status-switching set (LPS, SPM, SSM, SVC,
  TS). Each is implemented from its Principles-of-Operation page and
  covered by tests asserting encoding, effect, and condition code —
  including the carry/overflow indicator rules that existing emulators
  skip.
- Program-exception and SVC interrupts: real PSW swaps through the
  preferred storage area (old/new PSW pairs at 0x48/0x4C and 0x58/0x5C),
  interrupt codes, privileged-instruction checking, wait state, and
  register-set switching on interrupt entry.
- Full RR/SRS/RS/RI/SI decode of the §13 op-code assignment tables
  (unassigned encodings are illegal-instruction traps).
- Effective-address generation: SRS (with fullword displacement
  scaling), RS extended, RS indexed with IC-relative ±, halfword
  indirect, postindexed indirect, and automatic index modification;
  expanded 19-bit addressing via the BSR/DSR/DSE sector registers;
  automatic index alignment with the LM/STM exception.
- Two register sets of eight 32-bit GPRs, 64-bit PSW, 2-bit condition
  code (00 zero / 01 positive / 11 negative), sticky overflow indicator.
- A tiny assembler, a golden-trace harness, and a CLI runner.

## What doesn't (yet — all trap rather than guess)

- Machine-check/system/timer interrupts, storage protection, I/O (IOP),
  the stack and DSE-loading instructions (SCAL/SRET, LXA/LDM...), MVH,
  and the fullword-indirect addressing modes.
- Timing: this is an instruction-level emulator, not cycle-accurate.

See [docs/ISA_STATUS.md](docs/ISA_STATUS.md) for the per-instruction
verification table (and open questions — nothing here is guessed),
[docs/SOURCES.md](docs/SOURCES.md) for the primary sources and what each
confirmed, [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for design and
language rationale, and [docs/PRIOR_ART.md](docs/PRIOR_ART.md) for how
this relates to the Virtual AGC ecosystem's Shuttle work.

## Build and test

Requires a stable Rust toolchain.

```
cargo build
cargo test
```

`cargo test` runs the whole suite (135 tests: unit, per-instruction,
addressing-mode, and golden-trace) and reports pass/fail per test.

## Run a program

```
cargo run --bin lazap -- examples/sum.asm
```

prints a per-step architectural trace and the halt reason. Programs halt
by branching to themselves (`DONE: B DONE`); see `examples/sum.asm` for
the assembler syntax.

## Honesty statement

The AP-101's exact instruction set is not casually available; it is
reconstructed here from the manufacturer's Principles of Operation with
every implemented behavior cited, cross-checked against the Virtual AGC
project's simulator work, and every gap or source conflict recorded in
ISA_STATUS.md rather than papered over. If you find a behavior this
emulator gets wrong against period documentation or hardware evidence,
that's a bug — please report it with the source.
