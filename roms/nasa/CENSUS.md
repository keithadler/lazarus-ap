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
