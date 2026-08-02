# Roadmap

Phase 1 (done) built a verified single-CPU AP-101S emulator. The long-term
goal is the thing no public project has attempted: a **synchronized,
voting redundant set of emulated GPCs with crew keyboard/display input**,
faithful to the Shuttle DPS architecture. Prior-art boundary: see
[PRIOR_ART.md](PRIOR_ART.md) — single-GPC execution exists elsewhere;
multi-GPC sync/voting and DEU keyboards do not.

A design constraint that shapes everything below: on the real Shuttle,
"voting" was (1) redundancy-management *software inside PASS* running on a
lockstepped redundant set exchanging sync codes, and (2) hydraulic force-
voting at the actuators. PASS itself is ITAR-restricted, so Lazarus AP
builds the *mechanisms* (buses, sync, fault injection, force-vote model)
and demonstrates them with our own flight-style test software.

## Phase 2 — complete the processor (current)

- Floating point: IBM hexadecimal short/long formats and the full §8
  instruction set (AE/SE/CE/ME/DE families, loads/stores, CVFX/CVFL,
  MVS, LFLI/LFLR/LFXR), with §8.7 condition-code and §8.8 exception
  rules; cross-checked against yaGPC2's floatIBM implementation.
- Interrupt system: PSW swap via the §2.5.2 preferred-storage-area
  old/new PSW pairs; program-exception codes (illegal op, privileged op,
  fixed/floating overflow, underflow, significance); supervisor call;
  wait state. Machine-check and external interrupts stubbed until I/O.
- Storage protection + instruction monitor (§2.4) as far as CPU-visible.
- Remaining CPU instructions: MVH, TS, TSB, SPM, SSM, LPS, SVC, SCAL,
  SRET, LXA/LXAR, LDM, STXA/STXAR, STDM, ISPB; fullword-indirect
  addressing modes (Figure 2-17).

Exit criteria: every §12.1 instruction either executes with a citation or
is I/O-dependent (PC, ICR, DIAG); interrupt round-trip tests pass.

## Phase 3 — I/O processor and buses

- IOP model: 24 serial data buses, BCE/MSC instruction sets (the full
  PDF's IOP sections + nsts-sim-gpc's IOP as cross-check; port or
  interoperate with attribution rather than duplicate).
- Program-controlled I/O (PC instruction), DMA, timers (the §17 timing
  data becomes relevant here for bus scheduling realism).
- Bus abstraction designed for *multiple* bus controllers/listeners from
  day one — this is the seam phase 4 plugs into.

## Phase 4 — the redundant set (the new ground)

- N `Cpu` instances (target 4 PASS + 1 BFS-style listener) with:
  - inter-computer communication (ICC) buses,
  - sync discretes and a common-set/redundant-set synchronization model,
  - flight-critical bus listening (each GPC hears the others' commands),
  - a force-vote actuator model (outlier loses),
  - deterministic scheduling + fault injection (kill a GPC, corrupt its
    memory, skew its clock) so divergence and failover are testable.
- Deliverable demo: our own redundancy-management test software running
  identically on 4 emulated GPCs, detecting an injected fault, voting
  the sick machine out, annunciating it — end to end, reproducibly.

## Phase 5 — crew interfaces

- DEU keyboard/display model on the display buses: keystroke encoding,
  display formats per the public DPS Overview Workbook / DPS Dictionary.
- A terminal (later graphical) front end: type on the keyboard, watch
  the (test-software) displays, flip a GPC's mode switch.

## Phase 6 — flight software (legal-path dependent)

- Object-code compatibility with the Virtual AGC toolchain (ASM101S,
  lnk101) so their artifacts run here.
- Fragmentary BFS source (OI34.01) is publicly available and worth
  studying; running PASS/full BFS depends on ITAR status and is
  explicitly out of scope until that changes.
