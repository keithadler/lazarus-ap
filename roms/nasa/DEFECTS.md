# Defect report — AP-101S flight software stack

Filed while resurrecting the machine, in the format you would use today.
Items 1-3 are defects in the flight hardware, documented by IBM itself
under headings reading ANOMALY NOTE — shipped, flown for thirty years,
and still open. Item 4 is a defect in the flight software's error
handling. Item 5 is in the recovered tooling. The appendix holds this
project's own bugs, which do not belong alongside the flight system's.

## AP-101S-1 · Floating-point COMPARE reports false equality · HIGH
Component: CPU floating-point compare (CE/CER/CED/CEDR).
Status: documented by IBM in the Principles of Operation, §8.11 "ANOMALY
NOTE" — an admission, not a discovery.
Description: two unequal numbers compare EQUAL when their fractions differ
by exactly X'80 0000' after prealignment. The manual prints three worked
examples of its own hardware getting it wrong.
Impact: flight code branching on equality of nearly-identical scalars can
take the wrong branch.
Reproduce: execute CED on the manual's operand pairs, e.g.
423F FFFF 0000 1234 vs 423F FFFF 0080 1234 → "equal"; correct is "less".
Note: Lazarus AP computes the CORRECT answer; the anomaly is catalogued in
docs/ISA_STATUS.md, deliberately not replicated.
Workaround: compare with a tolerance, not for equality.

## AP-101S-2 · A masked DMA interrupt destroys arithmetic error detection · HIGH
Component: CPU interrupt system, DMA store-protect handling.
Status: documented by IBM, Figure 2-20 ANOMALY note.
Description (IBM's words): a masked DMA store-protect interrupt "will
set the condition code (CC) to a binary 10 and clear the carry and
overflow bits. This can result in erroneous GPC operation if an
instruction tries to utilize the CC, carry bit or overflow bit before
they are set by another instruction." Further: it "clears any fixed
point overflow, floating point underflow, and floating point overflow
interrupts... This can result in a lost arithmetic interrupt if a
masked DMA store protect interrupt occurs during an instruction that
causes one of these arithmetic interrupts."
Impact: the most serious item here. An overflow that DID occur can be
erased before the program is told — arithmetic error detection that
silently fails, triggered by unrelated I/O activity. Condition codes
are corrupted mid-computation as well.
Reproduce: timing-dependent; needs a DMA store-protect violation inside
an instruction that raises an arithmetic interrupt.
Workaround: none documented.

## AP-101S-3 · Block move corrupts data at a specific address overlap · MEDIUM
Component: CPU, MOVE HALFWORD (MVH).
Status: documented by IBM, §9.4 ANOMALY NOTE.
Description: MVH "will not correctly move data when the expanded source
address is exactly one greater than the expanded destination address and
the most significant bit of R1 and R2 are not equal."
Impact: silent data corruption on a legal instruction with legal
operands; the programmer must remember the exclusion zone.
Workaround: the manual's own — move fullwords, offset by two halfwords.

## HALS-FC-1 · ON ERROR handlers never fire for a singular matrix · MEDIUM
Component: HAL/S-FC compiler + flight runtime matrix inverse (MM14S3).
Description: a program may register ON ERROR for the singular-matrix
condition (group 4, number 27) and the compiler accepts it, but the
handler can never run. User dispatch happens only because the compiler
emits its own re-check after the call; for matrix inversion it emits
none, since singularity cannot be cheaply re-tested. The routine falls
through to an identity-matrix fallback.
Impact: error handling that compiles, reads correctly, and is silently
dead; the program proceeds with an identity matrix believing it holds
an inverse.
Status: confirmed against the flight software's own SEND ERROR handler
("DELETE ON ERROR SUPPORT", 1978) by the Virtual AGC project. This
emulator implements the exclusion, so the dead handler stays dead here.

## AP-101S-4 · Long DIVIDE silently loses half its precision · MEDIUM
Component: CPU double-precision divide (DED/DEDR).
Status: documented by IBM, §8.15 ANOMALY NOTE; cause not characterized.
Description: "under certain conditions, the accuracy of the quotient is
limited to 29 fractional bits (counting 1 to 56)... it is not feasible to
characterize these conditions", with the advice not to use long divide
where more than 29 bits are required.
Impact: silent precision loss with no indication to the program.
Reproduce: not reproducible on demand — that is the defect.
Workaround: the manual's own; avoid long divide when precision matters.

## ASM101S-1 (recovered tooling, not a flight article) · Three flight routines no longer assemble · MEDIUM (tooling)
Component: ASM101S (recovered modern port) + RUNMAC macro library.
Found: census of the flight runtime, 2026-08-02 — 202/205 assemble.
Failing: CSTRUC (2 errors), VV8D3, VV8S3 (1 each).
Diagnostic: (Pass -1, Severity 4) INVALID CC OPERAND FOR INTRINSIC, raised
by AEXIT when an intrinsic returns a condition code — yet VV8S3's own
header declares OUTPUT CC. Source and macro library disagree.
Impact: those three cannot be rebuilt from source today. The loss is in
the tooling, not the flight code.
Reproduce: ASM101S.py --library RUNASM/VV8S3.asm
Suspected cause: macro-library version skew; unconfirmed.

# Appendix — this project's own defects

## LAZARUS-1 · Emulator dropped the first halfword of every block move · FIXED
Component: Lazarus AP, MVH instruction.
Found by: a Shuttle physics program printing a garbage acceleration while
its two neighbouring components were bit-exact.
Cause: Figure 9-1 does not state whether the move count decrements before
or after each transfer. We guessed after; the hardware decrements first,
so every block copy lost element zero — in the HAL/S runtime's constant
table, the high half of 0.1.
Impact: wrong constants with no error indication. Precisely the class of
fault that voting computers exist to catch.
Reproduce: git show f24b937 (fix + regression test).
Filed because a resurrection that only lists other people's bugs is not
being honest.
