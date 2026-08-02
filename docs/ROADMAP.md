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

## Phase 4 — the redundant set (the new ground) — FIRST MILESTONE REACHED

Working today (tests/redundant_set.rs): four complete GPCs (CPU + IOP)
running one identical software image — CPU compute/poll/vote code, MSC
sequencer, BCE bus programs, configured only by GPC id — exchange
computed values over a shared serial-bus fabric via App. III §4 listen
mode and independently vote. An injected sensor fault on one GPC is
flagged by all three healthy machines (and the sick machine shows the
classic votes-against-the-world signature). Since then: CPU software
reaches its own IOP with real PC instructions (PCO/PCI); the redundancy
software has a poll-timeout protocol, so a killed GPC is voted out by
its silence (empty mailbox disagrees with everyone); sync discretes are
cross-wired GPC-to-GPC and a software barrier over them works (raise
own line via PCO, poll all lines via PCI); and a force-voted actuator
model taps the flight-critical buses — the outlier port is bypassed and
the surface follows the healthy command. Also working: a BFS-style
fifth GPC (all-listener, transmitter disabled) that shadows the
exchange invisibly and reaches the majority verdict; bus-level fault
injection (garbled SEV bits — receivers reject the words and the set
votes out the victim, who sees a healthy world, exactly the ambiguity
real cross-strapping addressed); and clock skew (a quarter-speed GPC),
absorbed by resumable MTO-governed reception (§3.4). Remaining for
full phase 4:

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

## Phase 5 — crew interfaces — STARTED (see DEU_STATUS.md)

Working: DEU model on a display bus (keystroke polling via #MIN,
display writes via #MOUT, BFS eavesdrop of crew input); protocol
encodings are documented emulator conventions pending the real DEU
spec. Remaining:

- DEU keyboard/display model on the display buses: keystroke encoding,
  display formats per the public DPS Overview Workbook / DPS Dictionary.
- A terminal (later graphical) front end: type on the keyboard, watch
  the (test-software) displays, flip a GPC's mode switch.

## Phase 7 — compile our own HAL/S — COMPILER RUNS

Done this session: XCOM-I (pure Python) translated the release-32V0
XPL source and clang built all seven passes (HALSFC-PASS1..PASS4, FLO,
OPT, AUXP, in PASS.REL32V0 itself); the HALSFC driver then compiled a
brand-new program — roms/lazarus/LAZARUS.hal, written for this
project — successfully: roms/lazarus/LAZARUS.obj is a genuine IBM
object deck (EBCDIC ESD/TXT/RLD cards, "$0LAZARU" entry, HAL/S-FC
version stamp). Compile recipe:
    export PATH="$PWD:$PWD/../../ported/PASS1.PROCS:<XCOM-I dir>:$PATH"
    python3 HALSFC LAZARUS.hal -o LAZARUS.obj

Remaining to close the loop — LINKING. `lnk101` exists nowhere public
(only on the author's machine; confirmed by full-tree search). Path
forward: write our own single-program linker against the object format
(ASM101S/readObject101S.py is the working format spec; objectWriter.py
the writer; the four fixture FCM+symbol-JSON pairs are layout ground
truth, including how the runtime library modules — IOINIT etc., from
RUNMAC — get placed). Open question: where lnk101 sources its RTL
objects (the fixture symbol JSONs' "modules" arrays list what it
linked in).

The recovered Shuttle-era HAL/S-FC compiler source (release 32V0, XPL +
BAL, Ron Burkey's ASCII restoration) lives in the Virtual AGC tree at
`yaShuttle/Source Code/PASS.REL32V0`, with a working Makefile: XCOM-I
translates the XPL to C, gcc compiles the seven passes, and the HALSFC
driver script runs them (plus a Python PASS1 port as a cross-check).
Recipe to fetch:
    git clone --depth 1 --filter=blob:none --sparse \
        https://github.com/virtualagc/virtualagc
    git sparse-checkout set "yaShuttle/Source Code/PASS.REL32V0" \
        yaShuttle/yaGPC2 yaShuttle/yaGpcIntegration
XCOM-I is elsewhere in the same tree (see the Makefile's references and
ibiblio.org/apollo/XPL.html). The `lnk101` linker's source location is
still to be found (tools.md documents its CLI: `lnk101 OBJ -o FCM
--json-symbols SYM`; check the yaHALMAT2 tree and sandroid.org). When
both build, the loop closes: write new HAL/S, compile with the real
flight compiler, run on Lazarus AP at byte parity.

## Phase 6 — flight software (legal-path dependent) — RUNTIME DONE

FCM loading works (src/fcm.rs): the Virtual AGC Shuttle toolchain is
HALSFC (HAL/S compiler) -> lnk101 (linker) -> .fcm memory image + a
symbols JSON carrying the entry point; yaGPC2 loads the image at
address 0 and starts the PSW at the entry (verified against its
ageharness.c/mcm.c). Lazarus AP now boots the same way. Remaining for
real HAL/S artifacts: the halucp layer (host-side interception of the
HAL/S runtime's WRITE/FILE SVCs using the symbol table) and validation
against an actual HALSFC/lnk101-built .fcm — the earlier ASM101S/
ibmobjdump note was wrong; no separate object-deck tooling is needed
for the yaGPC2-compatible path.

- Object-code compatibility with the Virtual AGC toolchain (ASM101S,
  lnk101) so their artifacts run here.
- Fragmentary BFS source (OI34.01) is publicly available and worth
  studying; running PASS/full BFS depends on ITAR status and is
  explicitly out of scope until that changes.
