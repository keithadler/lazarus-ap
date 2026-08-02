# Architecture

## Implementation language: Rust

Chosen over C and Python:

- The brief prefers a compiled, cycle-oriented-capable language. Rust
  gives C-class performance with checked integer-width conversions —
  and this codebase is *all* integer-width edge cases (16/19/32/64-bit
  quantities, fractional arithmetic, wrapping vs. overflow-detecting
  adds). `overflowing_add`, `wrapping_*`, and explicit `u16`/`u32`/`i64`
  casts make the PoO's bit-exact semantics auditable in a way C's silent
  promotions do not.
- `cargo test` gives the required one-command, per-test-reporting CI
  story with no extra harness.
- Exhaustive `match` on the instruction enum means an unhandled opcode is
  a compile error, which supports the project's honesty constraint: the
  decoder can only produce instructions the executor explicitly handles
  (or explicit `NotImplemented` traps).

Python was rejected as primary (reference-model-only per the brief); C
was a close second but offers no advantage here beyond ubiquity.

## Module layout

```
src/
  mem.rs     halfword-addressed big-endian storage (§2.1), configurable
             size, default 2^19 halfwords
  psw.rs     64-bit PSW fields + word0/word1 packing (§2.5.1 Fig. 2-19)
  decode.rs  bit-level decoder for the §13 op-code assignments
             (RR/RR-alt/SRS/RS/RS-alt planes, immediate groups, shifts)
  cpu.rs     machine state (2×8 GPRs, 8 FPRs, DSEs), effective-address
             generation (§2.2.5-2.2.9, §11.1, §14.1), and execution of
             the phase-1 subset; typed traps for everything else
  asm.rs     tiny two-pass assembler for readable tests (not a
             reconstruction of the historical assembler)
  trace.rs   golden-trace runner: deterministic per-step state lines
  bin/lazap.rs  CLI: assemble, run, print trace
tests/
  fixed_point.rs, branch.rs, shift_logical.rs   per-instruction tests
  addressing.rs                                 EA-mode + sector tests
  golden.rs + golden/*.txt                      end-to-end golden traces
```

## Key design decisions

- **Halfword-addressed memory.** The AP-101's smallest addressed unit is
  the 16-bit halfword (§2.1.1); `Memory` stores `Vec<u16>` and provides
  halfword/fullword/doubleword accessors plus a byte view that exists
  only as a host-side loader convenience (explicitly *not* an ISA
  feature).
- **16-bit IC + sector registers, expansion at use.** The PSW keeps the
  16-bit instruction counter exactly as the hardware does; every fetch
  expands it through the BSR (§2.2.9). Branches store 16-bit targets;
  BCRE/interrupt-free phase 1 means the BSR/DSR change only via BCRE.
- **Typed traps, no guessing.** Illegal encodings, unimplemented
  instructions (with mnemonic), the unimplemented fullword-indirect
  addressing modes, and out-of-range storage accesses all return `Trap`
  values. Where the manual says "indeterminate", the emulator makes a
  deterministic choice and documents it in ISA_STATUS.md.
- **Interrupts deferred, honestly.** The only phase-1-reachable
  interrupt (fixed-point overflow, PSW bit 20) halts with
  `Trap::FixedPointOverflow` instead of pretending to PSW-swap.
- **Halt convention.** The instruction set has no problem-state halt;
  `Cpu::run` stops when a taken branch targets itself (the standard
  test-program idiom), on wait state, on trap, or at a step budget.
- **Instruction-level, not cycle-level.** Timing (§16-17) is out of
  scope for phase 1; the structure (a `step()` that fetches, decodes,
  executes one instruction) leaves room for a timing model later without
  changing test semantics.

## Phase-2+ seams

- Floating point: `fpr` already exists on `Cpu`; §8 semantics (IBM hex
  float) slot into `exec` where the `NotImplemented` arms trap today.
- Interrupts/PSW swap: `Psw::word0/word1` + `set_word0/set_word1` are the
  PSW-image primitives an interrupt system needs.
- IOP/BCE/MSC (I/O), storage protection, DSE loading (LXA/LDM), and the
  fullword indirect pointer modes are all isolated behind today's traps.
- ROM loading: `roms/` is reserved for later phases (flight software is
  ITAR-restricted; see PRIOR_ART.md).
