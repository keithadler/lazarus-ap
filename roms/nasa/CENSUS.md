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
