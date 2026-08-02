# Defect report — AP-101S flight software stack

Filed while resurrecting the machine, in the format you would use today.
Two items are the original engineers' own documented admissions; one is a
live defect in a recovered tool; one was ours.

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

## AP-101S-2 · Long DIVIDE silently loses half its precision · MEDIUM
Component: CPU double-precision divide (DED/DEDR).
Status: documented by IBM, §8.15 ANOMALY NOTE; cause not characterized.
Description: "under certain conditions, the accuracy of the quotient is
limited to 29 fractional bits (counting 1 to 56)... it is not feasible to
characterize these conditions", with the advice not to use long divide
where more than 29 bits are required.
Impact: silent precision loss with no indication to the program.
Reproduce: not reproducible on demand — that is the defect.
Workaround: the manual's own; avoid long divide when precision matters.

## ASM101S-1 · Three flight routines no longer assemble · MEDIUM (tooling)
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
