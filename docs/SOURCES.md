# Sources

Every architectural fact in Lazarus AP traces to one of the sources below.
"§" references throughout the code and docs refer to **[1]** unless stated
otherwise.

## Primary sources

### [1] IBM 85-C67-001 — *Space Shuttle Model AP-101S Principles of Operation with Shuttle Instruction Set* (IBM Federal Systems Division)

The authoritative instruction-set reference for the AP-101S, written by the
machine's manufacturer. Two scans were consulted:

- Complete scan (~533 pages): hosted by the Virtual AGC project,
  <https://www.ibiblio.org/apollo/Shuttle/Shuttle%20GPC%20Software%20Model%20AP-101S.pdf>
- Partial scan (91 pages, typeset excerpts):
  <https://archive.org/details/GandalfDDI-SpaceShuttleDocuments> ("IBM
  AP-101S General Purpose Computer With Shuttle Instruction Set")

What it confirmed (sections cited in code/docs):

- **§1**: AP-101S is software compatible with AP-101C/M (IBM No. 6246156B).
- **§2.1.1-2.1.3**: 16-bit halfword transfer unit; fullword = 2 halfwords
  addressed by leftmost halfword; bits numbered 0 (MSB) upward; 19-bit
  halfword addressing; AP-101S drops the even-boundary alignment
  requirement of earlier AP-101 models.
- **§2.2.1**: **two sets of eight** 32-bit fixed-point general registers +
  one set of eight floating-point registers; 4-bit DSE per fixed register;
  register pairing is (R1, (R1+1) mod 8); PSW bit 44 selects the register
  set. Figure 2-2: base registers are GR0-GR3 only (B2 2-bit field; B2=11
  is GR3 for SRS, "none" for RS); index registers are GR1-GR7 (X=000 means
  no indexing).
- **§2.2.2**: fixed-point data is **fractional** twos complement (halfword
  = 15 bits + sign, fullword = 31 + sign, doubleword = 63 + sign).
- **§2.2.3-2.2.8**: instruction formats (RR, SRS, RS, SI, RI), field
  layout, "displacements of the form 111XXX are not valid" for SRS,
  SRS halfword/fullword displacement alignment, RS extended (AM=0,
  16-bit displacement, B2=11 = no base) and indexed (AM=1, X/IA/I,
  11-bit displacement) addressing including IC-relative, halfword
  indirect, postindexing, automatic index and storage modification, and
  the fullword indirect address pointer (Figure 2-17).
- **§2.2.9**: expanded (16 → 19 bit) addressing via DSR/BSR/DSE sector
  registers.
- **§2.5.1** (Figure 2-19): the 64-bit PSW field layout (IC 0-15, CC
  16-17, carry 18, overflow 19, masks, BSR 24-27, DSR 28-31, register
  select 44, wait 46, problem state 47), and "the machine architecture
  makes provision to address 262,144 fullwords, and the AP-101S ...
  provides full addressing capability".
- **§4-§7**: per-instruction descriptions — operation, resulting condition
  code, indicator (carry/overflow) behavior, and program interruptions for
  every fixed-point, branching, shift, and logical instruction implemented
  here. Notably: CC encoding 00 = zero/equal, 01 = positive/greater,
  11 = negative/less (10 unused by these instructions); loads set the CC;
  the overflow indicator is sticky and cleared by BVC-family instructions.
- **§8**: floating-point format — IBM hexadecimal float: sign, 7-bit
  excess-64 characteristic, hex fraction (short: 24-bit fraction in a
  fullword; long: 56-bit fraction in a register pair); CC set by FP
  add/subtract/compare/convert/load/midvalue-select but not by
  multiply/divide/store. (Documented; execution not implemented in
  phase 1.)
- **§11.1**: effective-address generation summary chart.
- **§12.1**: the complete Shuttle instruction set catalog (mnemonics and
  formats) — the coverage checklist for ISA_STATUS.md.
- **§13**: op-code assignment tables — the bit-level encoding map used by
  the decoder, including the RR/RS "alternate" planes (bit 12), the
  implied (op 10100) and explicit (op 10110) immediate groups with R1 as
  op-code extension, and the op 11001 group (STM/SVC/LM/LPS/SPM).
- **§14.1**: automatic index alignment (halfword/fullword/doubleword), and
  the LM/STM/LPS/ISPB halfword-alignment exception.
- **§16**: pipeline behavior (background only; not modeled).
- **§17**: instruction execution times (not modeled in phase 1).

### [2] Virtual AGC project — Space Shuttle pages

<https://www.ibiblio.org/apollo/Shuttle.html> and the document library
<https://www.ibiblio.org/apollo/links-shuttle.html>.

Confirmed: the document inventory above (including IBM 6246156B, *AP-101
C/M Principles of Operation*, and IBM 75-A97-001, the 1975 AP-101 CPU
technical description, for future AP-101B work); AP-101B memory 416KB vs
AP-101S 1024KB (CMOS, flown from STS-37, 1991); the survey of
reconstruction efforts recorded in PRIOR_ART.md.

### [3] yaGPC2 (Virtual AGC repository) — cross-check only

<https://github.com/virtualagc/virtualagc/tree/master/yaShuttle/yaGPC2> —
a C port of Don Schmidt's AP-101S simulator (see PRIOR_ART.md). Used to
**cross-check** the §13 encoding tables (its per-instruction bit-pattern
strings agree with our decoder for every implemented instruction) and to
resolve details the scan's OCR left ambiguous (e.g. the shift-type bits,
BALR's R2=0 behavior, condition-code numeric values 0/1/3). No code was
copied; divergences from it are deliberate and documented in
ISA_STATUS.md (XUL, BALR link ordering, carry/overflow modeling).

## Secondary sources (context, no ISA facts taken)

- NASA, *Computers in Spaceflight: The NASA Experience* (Tomayko), ch. 4 —
  historical background on the AP-101 and the DPS. (The old
  history.nasa.gov mirror now redirects to NTRS citation 19880069935.)
- ColanderCombo/nsts-sim-gpc README — status of the upstream simulator.

## What was *not* available

- A clean machine-readable AP-101S PoO — both scans are OCR'd paper; every
  numeric fact used here was read from the page text and, for encodings,
  double-checked against §13 and [3].
- Authoritative AP-101B (pre-upgrade) instruction-set documentation is
  *available* (IBM 6246156B, 75-A97-001 scans) but has not yet been read;
  Lazarus AP therefore currently targets the **AP-101S** and makes no
  claims about AP-101B-specific behavior.

## Differential verification (2026-08-02)

Output parity proves the answers match. `tools/difftest.py` proves the
machines agree *step by step*: it runs yaGPC2 with `--trace` and Lazarus
AP with `lazap-trace` on the same image and compares every executed
instruction - address, opcode, and every register the reference reports
changing.

    hello.fcm             IDENTICAL   6000 instructions
    176-P.fcm             IDENTICAL    534 instructions
    LAZARUS.fcm           IDENTICAL    401 instructions
    read_eof_onerror.fcm  IDENTICAL     28 instructions

That sweeps the instruction set as actually exercised by real
flight-compiler output, rather than trusting that a handful of programs
happen to cover it. Two harness bugs were found and fixed while
building it (both ours, not the emulator's): our trace samples
registers before each instruction while the reference reports what an
instruction produced, so comparison is offset by one step; and the
reference's section names can run straight into the offset field
("#CREADAC+0000:"), which a whitespace-hungry parser silently drops.
