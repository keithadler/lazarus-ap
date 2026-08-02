# Prior art

Survey of existing Space Shuttle GPC software-reconstruction efforts,
what each achieved, where each stopped, and what Lazarus AP does
differently. Primary index: the Virtual AGC project's Shuttle pages
(<https://www.ibiblio.org/apollo/Shuttle.html>).

## Emulators

### nsts-sim-gpc (Don Schmidt) — the furthest-along AP-101 simulator

<https://github.com/ColanderCombo/nsts-sim-gpc>. An instruction-level
simulator of the AP-101 B and S models in CoffeeScript/TypeScript
(Node/Electron GUI debugger, a.k.a. *gps-batch*/*gps-gui* on the Virtual
AGC page), including a basically-tested IOP model for the 24 serial
buses. Explicitly "makes no attempt to simulate timing, microcode, or
internal state". No formal test suite is visible in the repository.

### yaGPC2 (Virtual AGC repository)

<https://github.com/virtualagc/virtualagc/tree/master/yaShuttle/yaGPC2>.
A C port of the above (the sources cite `gpc/cpu_instr.coffee` line by
line). Same scope and same gaps as its upstream: notably, its
arithmetic does **not** model the carry/overflow indicators (its
`cpu_compute_cc_arith` sets only the condition code), and its XUL is an
XOR-combine rather than the exchange the PoO describes.

### Dan Weaver's AP-101S emulator

Mentioned on the Virtual AGC page; no code or documentation published.
Status unknown.

## Compiler / language reconstruction (different layer than Lazarus AP)

- **HAL/S-FC recovery**: the original HAL/S flight compiler (release
  32V0, XPL/I + BAL + AP-101S assembly) was recovered; Virtual AGC built
  **XCOM-I** to compile its XPL/I since the original Intermetrics XPL/I
  compiler did not survive.
- **yaHALMAT** (Zane Hambly): interprets HALMAT — the HAL/S compiler's
  intermediate language — bypassing the AP-101 code-generation passes
  entirely.
- **ASM101S** (assembler, work in progress), **lnk101** (linker),
  **ibmobjdump** (object-file viewer): the developing toolchain around
  AP-101S object code.

## Flight software

PASS source (OI34.06, OI30.17) and a GPC disassembly (OI34.07) exist but
are **ITAR-restricted**; fragmentary BFS source (OI34.01) is available.
This is why Lazarus AP's phase 1 deliberately targets the CPU + a test
suite rather than "load the flight software": the legally-clear path is
building a verified machine first.

## Where existing efforts stopped, and what Lazarus AP adds

Existing emulators (nsts-sim-gpc / yaGPC2) already execute AP-101S code
and were invaluable as an encoding cross-check. What they do not
provide, and what phase 1 of Lazarus AP contributes:

1. **A per-instruction verification trail.** No prior effort publishes a
   statement of which instruction behaviors are backed by which primary
   source. ISA_STATUS.md + SOURCES.md make the reconstruction itself a
   citable artifact, and the two source conflicts found while building it
   (XUL semantics; BALR link ordering for R1=R2) are recorded rather than
   silently inherited.
2. **An instruction-level test suite.** ~90 integration tests assert
   encoding, register/memory effect, and condition code for every
   implemented instruction, plus golden-trace end-to-end runs. Prior
   emulators have no committed test corpus a re-implementation can run
   against; Lazarus AP's tests are designed to be that corpus.
3. **Indicator (carry/overflow) semantics.** The PoO's carry/overflow
   rules — including subtraction's borrow convention, LCR's zero-only
   carry, SLL's last-bit-out carry, BVC's overflow clearing, and the
   sticky overflow indicator — are implemented and tested; yaGPC2 models
   none of them. Flight code branches on these (BVC exists for a
   reason), so they matter for eventually running real software.
4. **Typed refusal to guess.** Unknown encodings, unimplemented
   instructions, and unimplemented addressing modes trap with precise
   diagnoses instead of approximating.

Deliberately *not* duplicated: the IOP/bus model (nsts-sim-gpc's exists;
phase 2+ should interoperate or port with attribution), the HAL/S
toolchain (Virtual AGC's lane), and the GUI debugger. The intended
convergence point is ASM101S/lnk101 object-code compatibility, so
Lazarus AP can eventually run artifacts the Virtual AGC toolchain
produces.
