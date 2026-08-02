# IOP implementation status (phase 3, in progress)

Sources: the AP-101S scan bundles three IOP documents — **Appendix I**
(PCI/PCO Principles of Operation, the CPU↔IOP command interface),
**Appendix II** (Master Sequence Controller PoO), **Appendix III** (Bus
Control Element PoO). Cross-check: nsts-sim-gpc/yaGPC2's
`iop.c`/`iop_msc_instr.c`/`iop_bce_instr.c`.

## Implemented

- **CPU seam**: the PC instruction (main PoO §3.3) drives a pluggable
  `IoSubsystem`; `Iop` implements it.
- **Command word format** (App. I p. I-2): bit 0 PCO/PCI, bits 1-5
  one-hot subsystem select (CM/RM/DF/LS/CC), bit 6 handshake, bits 7-16
  data select. VERIFIED.
- **Command subset** (App. I p. I-4/I-5 summary tables, exact hex):
  PROCESSOR ENABLE/HALT (8720/8620), MASTER RESET (8440), MIA XMTR/RCVR
  ENABLE/DISABLE (8504/8404/8508/8408), DISCRETE OUTPUT SET/RESET
  (8510/8410), ENABLE INTERRUPTS (8814), LOAD MSC BUSY (9204); PCI
  PROCESSOR HALT STATUS (040C), READ DISC. OUTPUT STATUS (0408), READ
  STATUS 4 B/W (1004), INTERRUPT REG A-E (0800-0810). Unimplemented
  commands time out (CC 01) — documented convention.
- **Register model**: MSC (32-bit ACC, 18-bit X/PC per App. II §1.1-1.2;
  addresses are 18-bit with bit 17 as halfword selector), 24 BCEs (PC,
  base, busy, indicator per App. III §1.2), busy/wait register layout
  (MSC bit 0, BCE n at bit n+1).
- **System-class interrupts in the CPU** (main PoO §2.5.2): pending
  per-level delivery through the PSA pairs 0060-009C, remain-pending
  while masked, lowest-mask-bit-first priority, interruptible wait
  state. The IOP's INTERRUPT CPU path will raise external level 2
  (0088/008C, mask bit 37).

## MSC execution engine — implemented

The full App. II §3 repertoire executes, from the per-instruction pages
(II-27..II-95) with encodings cross-checked against yaGPC2:
accumulator/memory (@L/@A/@N/@X/@ST, @LF/@LH/@STF/@STH with the T
halfword bit and sign extension), branches (@BC/@BXC condition codes,
@BU with all four Table 1.2 modes, @CALL with the delta return-address
convention, @REC full state reload), skips (@TSZ, @CI/@C, @TMI/@TM),
BCE loads (@LBB/@LBP with the waiting-only rule and status error bits
12/13), register ops (@LAR, @SFD/@RFD, @LMS status word, @SIO with the
already-busy error, @XAX, @SEC external-call save/branch via local
store C6, @RBI), the eight register immediates, the four repeats, and
@WAT/@DLY/@INT/@STP. Short-format EA is PC-relative ±11-bit, optionally
indexed; long formats require even alignment (boundary error stops the
MSC, §2.4); illegal opcodes set the status bit and stop.

Documented deviations/choices:
- **@CI/@C "greater" increment**: the document says PC +2/+3/+4 for
  less/equal/greater; yaGPC2 uses +1 for greater. The document is
  followed.
- **@TXI skip condition**: the scan's glyph is ambiguous ("< 0" vs
  "≤ 0"); ≤ 0 per the yaGPC2 cross-check.
- **Repeats and @DLY**: timing (33 µs minor loops, repeat counts) is
  not modeled; the condition is evaluated once (met = skip, else fall
  through). Correct once BCEs run concurrently; revisit then.
- **@INT**: the raw level field is exposed (`cpu_interrupt`) for the
  host to route to a CPU external interrupt; the level-encoding page
  needs a better read.

## Staged (encodings catalogued, execution NOT implemented)

BCE instruction set (App. III §3: #LTO/#RIB/#SIB/#SSC/#SST/#LBR/#BU/
#WIX/#CMD/#TDS/#TDL/#MOUT/#RDS/#RDL/#MIN etc.) and the bus/listen-mode
model (App. III §4) — the transmit/receive layer and the multi-GPC
seam. Encodings recorded from the yaGPC2 cross-check; per-page reads
(App. III pages III-30..III-88) come first, as with the MSC.

## Not yet modeled

DMA arbitration/timing, MIA bus electrical behavior, go/no-go timers,
local store, RM/DF subsystem internals, ICR/DIAG instructions, listen
mode (App. III §4 — the phase-4/5 seam for multi-GPC buses and
keyboards).
