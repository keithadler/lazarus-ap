# Resurrection census — the genuine Shuttle flight runtime (RUNASM)

2026-08-02: all 205 recovered AP-101S assembly routines of the HAL/S
flight runtime library were fed through the real ASM101S assembler
(with its macro library):

    ASSEMBLED CLEANLY: 202 / 205

Failures (to investigate): CSTRUC, VV8D3, VV8S3.

The library includes the Shuttle's own SQRT, sine/cosine (SNCS/DSNCS),
EXP, LOG, TAN, rounding, the full MM*/VV*/VR*/MR* matrix-vector suite
that steered the orbiter, and the character/IO runtime. SQRT.obj is
banked here; DRIVER.asm (the harness that will call it by the genuine
ABAL convention - BAL 4 through a #Q Figure-2-17 pointer stub, return
via AEXIT's BCRE) currently has 4 assembly errors to debug (DC Y()
expression syntax).

Stub-shape ground truth (from hello.fcm): #QCOUT = 8101 0E20 - hi =
sector-form entry address, lo = Xc|C|CB set + BSV sector nibble.

## SQRT execution status (2026-08-02, session end)

SQRTRUN.fcm (DRIVER + SQRT, linked standalone at 0x100/0x122) RUNS THE
COMPLETE FLIGHT ALGORITHM: trace shows the exact published sequence -
LER/BCF/LFXR/XR/SLDL/SLL (unpack), the Q=1 branch taken correctly for
2.0, SRA/A/L/DR/A/AR/LFLR (hyperbolic seed), DER/AER + LE/MER/DER/NHI/
A/LFLR/AER/SER/MER/AER (two Newton-Raphson passes), then AEXIT's
LH + BCRE returning to the driver, STE, SVC. Control flow is perfect.

BUG: `A R7,C` adds zero - the R1-based (USING A,R1) constant fetches
miss the #LSQRT table. Prime suspect: SRS fullword displacement
scaling - our emulator doubles the 6-bit displacement per section
2.2.5 Figure 2-8 (manual-verified); if ASM101S emits the displacement
already in halfwords for USING-based operands, the fetch lands at
A + 2d instead of A + d. Next: disassemble the emitted `A R7,C`
halfword from SQRTRUN.obj (SQRT csect, the instruction ~byte 0x22),
compare d against (C-A)=8 bytes, and check a real HALSFC listing or
yaGPC2 (--start flag) for the authoritative encoding. Also verify the
LA R1,A relocation target lands on #LSQRT's placement.

Trace: cargo run --bin lazap-trace -- roms/nasa/SQRTRUN.fcm

## ✓ FIRST FLIGHT ROUTINE RESURRECTED (same session, later)

SQRT computed sqrt(2.0) = 0x4116A09E — EXACT to the last bit.
The bug was in lnk_lite, not the flight code and not the emulator:
ASM101S adcons are CSECT-CHAIN-relative (they already include the
target CSECT's ESD-declared offset within its module), so the fixup is
placed_base - chain_offset (ESD bytes 9-12). With that, the 1970s
algorithm produced the ideal IBM hexfloat on the first run. Also:
DC F'0' fullword-aligns, so RESULT sits at driver+0xE.

Test: nasa_flight_sqrt_computes_sqrt2 (no longer ignored).
Recipe now proven for the other 201 assembled routines.

## For the record

On August 2, 2026 at 12:09 PDT, Keith Adler ran RUNASM/SQRT.asm on
Lazarus AP: sqrt(2.0) = 0x4116A09E, exact. Atlantis closed out the
Shuttle program on July 21, 2011; as far as this project can
establish, these instructions had scarcely executed anywhere since -
and never before in this resurrection.

## The flight math lab — resurrection log

| Routine | Convention | Input | Flight answer | Modern value | Status |
|---|---|---|---|---|---|
| SQRT | intrinsic (BAL 4) | 2.0 | 0x4116A09E = 1.4142135 | 1.4142136 | ✓ exact |
| SNCS sin | intrinsic | 0.5 rad | 0x407ABBA1 = 0.4794255 | 0.4794255 | ✓ |
| SNCS cos | intrinsic (F2!) | 0.5 rad | 0x40E0A940 = 0.8775826 | 0.8775826 | ✓ |
| EXP | LIB (ACALL/SRET) | 1.0 | 0x412B7E15 = 2.7182817 | 2.7182818 | ✓ |
| LOG | LIB | 2.0 | 0x40B17219 = 0.6931472 | 0.6931472 | ✓ |
| TAN | LIB | 0.5 rad | 0x408BDA7A = 0.5463024 | 0.5463025 | ✓ |

Conventions proven: intrinsics (AMAIN INTSIC=YES) called by plain
BAL 4 with an R0 frame; LIB routines (plain AMAIN) called by the
genuine ACALL sequence - DC X'D0FF' (SCAL 0) + Y(#Qname+X'3800')
through a hand-built Figure 2-17 stub (Y(entry) + X'0E00'), returning
via SRET. Driver template: EXP_DRV.asm; swap EXTRN/entry/argument.
195 more routines await the same treatment.

## ✓ Wave 2 RESURRECTED (SINH/TANH/ACOS/ASINH/ATANH)

| Routine | Chain | Input | Flight answer | Modern | Status |
|---|---|---|---|---|---|
| SINH | →EXP | 1.0 | 1.1752014 | 1.1752012 | ✓ |
| TANH | →EXP | 1.0 | 0.7615942 | 0.7615942 | ✓ |
| ACOS | →SQRT | 0.5 | 1.0471973 | 1.0471976 | ✓ |
| ASINH | →SQRT,LOG | 1.0 | 0.8813735 | 0.8813736 | ✓ |
| ATANH | →LOG | 0.5 | 0.5493059 | 0.5493061 | ✓ |

Each calls the genuine flight routines beneath it — SINH's answer is
computed by the Shuttle's own EXP; ASINH's by its SQRT and LOG.

Two linker discoveries closed this out:
1. ESD cards declare the ESDID of their FIRST entry (bytes 14-15) and
   hold up to three; numbering must start there, not from a running
   counter (they agree only for single-CSECT decks).
2. **lnk101 SYNTHESIZES the #Q call stubs.** hello.fcm carries
   #QCOUT/#QHOUT/#QIOINIT as standalone 2-halfword sections precisely
   because the linker manufactures one - Y(entry) + control halfword
   0x0E00 - for every unresolved #Q external. lnk_lite now does the
   same, so a driver need only EXTRN #QNAME and ACALL through it.

### (historical) the bug that led there

Dependency-closed drivers now build automatically: the tool reads each
census deck's #Q externals, walks the closure (SINH->EXP,
ASINH->SQRT+LOG), emits one #Q stub CSECT per LIB callee, assembles,
concatenates every deck, and links. All five produce images.

lnk_lite also now places each MODULE contiguously honoring its CSECTs'
declared chain offsets (ESD bytes 9-12) instead of scattering them -
required as soon as a module has more than one CSECT.

REMAINING BUG: in the driver deck, the stub's `DC Y(SINH)` relocates to
the module base (0x0100) instead of SINH's placed address (0x0164), so
SCAL loops back into the driver. Since the same relocation works for
single-CSECT decks, the suspect is ESDID->entry mapping in parse_deck:
ESD cards may carry blank/continuation slots (size field counts 16-byte
entries; a card can hold 3), so enumerating entries in parse order can
drift from the assembler's ESDIDs. Fix: read each ESD card's `esdid`
field (bytes 14-15 = the ID of its FIRST entry) and number entries from
there, rather than a global running counter.

## The vector suite reached (the code that actually flew)

| Routine | What it is | Convention | Input | Answer |
|---|---|---|---|---|
| VV6S3 | dot product | R2, R3 = vector ptrs (one fullword before), F0 out | [1,2,3]·[4,5,6] | 32 exact |
| VV10S3 | UNIT VECTOR | R4 = in ptr, R2 = out ptr; ACALL; needs VV0SN + SQRT | [3,4,0] | [0.6, 0.8, 0] |

Vector elements sit at displacements 2/4/6 off the pointer, so the
pointer again addresses one fullword before element 1 - the same
pre-increment convention the array routines use. VV10S3 normalizes by
calling the flight SQRT: pointing math, all the way down.
| MM6S3 | 3x3 MATRIX MULTIPLY | R2, R3 = matrix ptrs, R1 = result ptr | [[1,2,3],[4,5,6],[7,8,9]] x 2I | [2,4,6,8,10,12,14,16,18] |

Matrices are stored row-major as nine consecutive fullwords, same
one-fullword-before pointer convention. MM6S3 is the transform at the
heart of attitude work: every rotation the orbiter computed passed
through a routine shaped like this one.
