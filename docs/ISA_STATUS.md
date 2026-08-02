# ISA implementation status

Target machine: **IBM AP-101S** ("Shuttle instruction set"). All section
references (§) are to IBM 85-C67-001; see [SOURCES.md](SOURCES.md).

## Legend

| Status | Meaning |
|---|---|
| **VERIFIED** | Implemented. Encoding taken from the §13 op-code assignment tables and cross-checked against yaGPC2's decoder; semantics implemented from the instruction's own PoO page (§4-§7); covered by at least one test asserting encoding, effect, and condition code. |
| **PARTIAL** | Implemented and tested as above, but with a documented deterministic choice where the manual says "indeterminate", or a documented conflict between sources. |
| **NOT IMPLEMENTED (encoding verified)** | The decoder recognizes the §13 encoding and execution traps with the mnemonic; semantics not implemented in phase 1. |
| **UNVERIFIED** | No primary-source confirmation; nothing is implemented or guessed. |

Nothing in this emulator is fabricated: every implemented opcode, field
layout, condition-code rule, and indicator rule cites a PoO section, and
everything else traps.

## Machine model (all VERIFIED)

| Fact | Source |
|---|---|
| 16-bit halfwords, 32-bit fullwords, big-endian bit numbering, 19-bit halfword addressing | §2.1.1-2.1.2 |
| 262,144 fullwords (2^19 halfwords) fully addressable on the AP-101S | §2.5.1.1 |
| No even-boundary alignment requirement (AP-101S; earlier models differ) | §2.1.3 |
| Two sets of **eight** 32-bit general registers, PSW bit 44 selects the set; eight 32-bit floating-point registers; 4-bit DSE per GPR per set | §2.2.1 |
| Bases = GR0-GR3 (B2 field); indexes = GR1-GR7 (X=0 → no index); pairs are (R1,(R1+1) mod 8) | §2.2.1 Fig. 2-2 |
| Fractional twos-complement fixed-point data | §2.2.2 |
| RR/SRS/RS/SI/RI formats; SRS displacement 111XXX invalid; SRS fullword displacement scaling; RS extended/indexed addressing incl. IC-relative, indirect, postindex, auto index modification | §2.2.3-2.2.8, §11.1 |
| Expanded addressing: DSR (data, high bit 1), DSE (based, high bit 0), BSR (branches, IC-relative), implied 0000 | §2.2.9 |
| 64-bit PSW layout; CC = bits 16-17; carry = 18; overflow = 19 (sticky) | §2.5.1 Fig. 2-19 |
| CC values: 00 zero/equal/within, 01 positive/greater/all-ones, 11 negative/less/not-zero/mixed; 10 unused by fixed-point/logical ops | §4-§7 per-instruction pages |
| Automatic index alignment (half ×1 / full ×2), LM/STM/LPS/ISPB excepted | §14.1 |

**Correction to a common assumption:** the AP-101 does *not* have 16
general registers. The register fields are 3 bits; there are eight
registers per set, two sets (§2.2.1). It is also *not* binary-compatible
with System/360: the instruction formats are 16/32-bit with a 5-bit
primary op code — only the assembly-language style is S/360-derived.

## Fixed point (§4)

| Instr | Mnemonics | Formats | Status | Notes |
|---|---|---|---|---|
| Add | AR, A | RR, SRS, RS | VERIFIED | CC set; carry = carry-out; overflow sticky; fixed-point overflow interrupt honored as a trap (§4.1) |
| Add Halfword | AH | SRS, RS | VERIFIED | operand developed as high halfword + 16 zeros (§4.0/4.2) |
| Add Halfword Immediate | AHI | RI | VERIFIED | §4.3 |
| Add to Storage | AST | RS | VERIFIED | §4.4 |
| Compare | CR, C | RR, SRS, RS | VERIFIED | algebraic; indicators unchanged (§4.5) |
| Compare Between Limits | CBL | RR | VERIFIED | limits fullword = upper:lower; modifiers advance addresses after compare (§4.6) |
| Compare Halfword | CH | SRS, RS | VERIFIED | all 32 developed bits participate (§4.7) |
| Compare Halfword Immediate | CHI | RI | VERIFIED | §4.8 |
| Compare Immediate w/ Storage | CIST | SI | VERIFIED | CC is immediate-vs-storage (§4.9) |
| Divide | DR, D | RR, SRS, RS | PARTIAL | fractional divide, odd-R1 zero-appended dividend, CC unchanged, overflow incl. ÷0 (§4.10). Manual: registers "indeterminate" on overflow and (R1+1) "indeterminant" for even R1 — this emulator deterministically leaves them unchanged |
| Exchange Upper/Lower | XUL | RR | PARTIAL | implemented as the true exchange §4.11 describes. **Source conflict:** yaGPC2/nsts-sim-gpc implement XUL as a single XOR-combine (R1.upper' = R2.lower' = R1.upper ⊕ R2.lower), which is not an exchange; unresolved without hardware or flight-code evidence |
| Insert Address Low | IAL | SRS, RS | VERIFIED | EA (halfword-aligned) → R1 bits 16-31; RS forms live in the RS-alternate slot (§4.12, §13) |
| Insert Halfword Low | IHL | RS | VERIFIED | §4.13 |
| Load | LR, L | RR, SRS, RS | VERIFIED | **sets CC** (unlike S/360) (§4.14) |
| Load Address | LA | SRS, RS | VERIFIED | EA → bits 0-15, low zeroed, CC unchanged; B2=11/AM=0 = Load Halfword Immediate (§4.15) |
| Load Arithmetic Complement | LCR | RR | VERIFIED | overflow on −(−1.0 max neg); carry only for zero operand (§4.16) |
| Load Fixed Immediate | LFXI | RR(OPX) | VERIFIED | values −2..13 from 4-bit code (§4.17) |
| Load Halfword | LH | SRS, RS | VERIFIED | sets CC (§4.18) |
| Load Multiple | LM | RS | VERIFIED | 8 registers, ascending; halfword index alignment (§4.19) |
| Modify Storage Halfword | MSTH | SI | VERIFIED | CC set, indicators unchanged (§4.20) |
| Multiply | MR, M | RR, SRS, RS | PARTIAL | fractional (product = (a·b)≪1); even-R1 pair result; odd R1 keeps high half; CC unchanged; overflow only for (−1)×(−1) (§4.21) — for that case the manual gives no result value; this emulator stores the wrapped product (0x80000000…) |
| Multiply Halfword | MH | SRS, RS | VERIFIED | 16×16 → 32-bit fraction (§4.22) |
| Multiply Halfword Immediate | MHI | RI | VERIFIED | §4.23 |
| Multiply Integer Halfword | MIH | RS | VERIFIED | integer product → bits 0-15, low zeroed; overflow if it exceeds a halfword (§4.24) |
| Store | ST | SRS, RS | VERIFIED | CC unchanged (§4.25) |
| Store Halfword | STH | SRS, RS | VERIFIED | stores bits 0-15 (§4.26) |
| Store Multiple | STM | RS | VERIFIED | §4.27 |
| Subtract | SR, S | RR, SRS, RS | VERIFIED | a + ¬b + 1; carry = no-borrow (§4.28) |
| Subtract from Storage | SST | RS | VERIFIED | storage − R1 → storage (§4.29) |
| Subtract Halfword | SH | SRS, RS | VERIFIED | §4.30 |
| Tally Down | TD | SRS, RS | VERIFIED | storage halfword −1; CC set; indicators unchanged (§4.31) |

## Branching (§5)

| Instr | Mnemonics | Formats | Status | Notes |
|---|---|---|---|---|
| Branch and Link | BALR, BAL | RR, RS | VERIFIED | link = PSW word 0 (updated IC + status); target computed before linking; BALR R2=0 links without branching (§5.1) |
| Branch and Index | BIX | RS | VERIFIED | index +=1, count −=1, branch if old count > 0 (§5.2) |
| Branch on Condition | BCR, BC | RR, RS | VERIFIED | M1 bits test CC 00/11/01 (§5.3) |
| Branch on Condition Backward | BCB | SRS | VERIFIED | IC − disp (§5.4) |
| Branch on Condition (Extended) | BCRE | RR | VERIFIED | reloads IC + BSR + DSR from R2 (§5.5) |
| Branch on Condition Forward | BCF | SRS | VERIFIED | IC + disp (§5.6) |
| Branch on Count | BCTR, BCT | RR, RS | VERIFIED | count in bits 0-15; branch on non-zero result (§5.7) |
| Branch on Count Backward | BCTB | SRS | VERIFIED | §5.8 |
| Branch on Overflow and Carry | BVCR, BVC | RR, RS | VERIFIED | bit6 = carry, bit7 = overflow, bit5 inverts; clears overflow indicator (§5.9) |
| Branch on Overflow and Carry Fwd | BVCF | SRS | VERIFIED | §5.10 |

## Shifts (§6)

All VERIFIED: count field semantics (0 = no-op, 1-55 immediate, 56-63 =
bits 10-15 of GR0-GR7) per Figure 6-1; SLL/SLDL leave the last bit
shifted out in the carry; SRA/SRDA sign-fill; SRR/SRDR circular;
NCT normalizes until bit0 ≠ bit1 with the §6.1 zero/carry rules.

| Instr | Mnemonics | Status |
|---|---|---|
| Normalize and Count | NCT | VERIFIED |
| Shift Left Logical / Double | SLL, SLDL | VERIFIED |
| Shift Right Arithmetic / Double | SRA, SRDA | VERIFIED |
| Shift Right Logical / Double | SRL, SRDL | VERIFIED |
| Shift Right and Rotate / Double | SRR, SRDR | VERIFIED |

## Logical (§7)

| Instr | Mnemonics | Formats | Status | Notes |
|---|---|---|---|---|
| AND | NR, N | RR, SRS, RS | VERIFIED | CC 00 zero / 11 not zero (§7.1) |
| AND Halfword Immediate | NHI | RI | VERIFIED | §7.2 |
| AND Immediate w/ Storage | NIST | SI | VERIFIED | §7.3 |
| AND to Storage | NST | RS | VERIFIED | §7.4 |
| Exclusive-OR | XR, X | RR, SRS, RS | VERIFIED | §7.5 |
| Exclusive-OR Halfword Immediate | XHI | RI | VERIFIED | §7.6 |
| Exclusive-OR Immediate w/ Storage | XIST | SI | VERIFIED | §7.7 |
| Exclusive-OR to Storage | XST | RS | VERIFIED | §7.8 |
| OR | OR, O | RR, SRS, RS | VERIFIED | §7.9 |
| OR Halfword Immediate | OHI | RI | VERIFIED | §7.10 |
| OR to Storage | OST | RS | VERIFIED | §7.11 |
| Search Under Mask | SUM | RR | PARTIAL | §7.12; a non-positive count (undefined per the manual — "must be positive") deterministically performs zero iterations with CC=00 |
| Set Bits | SB | SI | VERIFIED | §7.13 |
| Set Halfword | SHW | SRS, RS | VERIFIED | CC **not** changed (§7.14) |
| Test Bits | TB | SI | VERIFIED | 3-state CC (§7.15) |
| Test Register Bits | TRB | RI | VERIFIED | §7.16 |
| Test Halfword | TH | SRS, RS | VERIFIED | §7.17 |
| Zero Bits | ZB | SI | VERIFIED | §7.18 |
| Zero Register Bits | ZRB | RI | VERIFIED | §7.19 |
| Zero Halfword | ZH | SRS, RS | VERIFIED | CC **not** changed (§7.20) |

## Floating point (§8) — implemented (phase 2)

IBM hexadecimal float: sign + 7-bit excess-64 characteristic + hex
fraction; short = 6 digits in a fullword, long = 14 digits in a register
pair (§8.1-8.5). Implemented with prealignment + guard digit, forced
true zeros, postnormalization, and the §8.8 exception rules (exponent
overflow → PE 000B with operands unchanged; underflow → PE 0009 when
masked on, true zero when off; significance → true zero always, PE 0005
when masked on; divide by zero fraction → suppressed, PE 000C). CC per
§8.7 (set by add/subtract/compare/convert/load/MVS, not by
multiply/divide/store; loads judge the fraction only and don't
normalize).

| Instr | Mnemonics | Status | Notes |
|---|---|---|---|
| Add / Subtract (short, long) | AER/AE, AEDR/AED, SER/SE, SEDR/SED | VERIFIED | §8.9/8.10/8.26/8.27 |
| Compare (short, long) | CER/CE, CEDR/CED | PARTIAL | correct algebraic compare implemented; the §8.11 ANOMALY (hardware returns false equality when prealigned fractions differ by exactly X'80 0000') is **not replicated** |
| Multiply (short, long) | MER/ME, MEDR/MED | PARTIAL | even-R1 short multiply fills the register pair (§8.25). Exact fraction product used; the hardware's partial-sum truncation for long multiply (three most significant fullword partial products, §8.24) may differ in the lowest-order bits |
| Divide (short, long) | DER/DE, DEDR/DED | PARTIAL | truncated quotient per §8.15/8.16; the §8.15 ANOMALY (long-divide accuracy limited to 29 fraction bits "under certain conditions") is not replicated — the manual says the conditions cannot be characterized |
| Load / Load Complement | LER/LE, LED, LECR | VERIFIED | no normalization; CC from fraction; LECR loads true zero for zero fractions (§8.17-8.19) |
| Store | STE, STED | VERIFIED | CC unchanged (§8.28/8.29) |
| Convert | CVFX, CVFL | VERIFIED | fixed point has the binary point between bits 15/16; CVFX CC on result bits 0-15; convert overflow → PE 000A (§8.13/8.14) |
| Midvalue Select | MVS | VERIFIED | limiter semantics and CC per §8.23; output normalized (may underflow) |
| Immediates / moves | LFLI, LFLR, LFXR | VERIFIED | LFLI table §8.21 |

## Interrupts and status switching (§2.5.2, §9) — implemented (phase 2)

PSW swaps through the preferred storage area (Figure 2-20/2-21):
program-exception class old/new at 0048/004C with codes 0000 illegal,
0001 privileged, 0004 fixed-point overflow (ENDOP check of PSW bits
19+20, incl. after SPM/LPS), 0005 significance, 0009 FP underflow,
000A convert overflow, 000B FP overflow, 000C FP divide; SVC at
0058/005C with the 16-bit EA as code and the sector extension in old-PSW
bits 40-43. Register-set switching via new-PSW bit 44 works. Emulator
convention (documented): an interrupt whose new-PSW doubleword is all
zero halts with a typed `UninitializedInterrupt` trap instead of
executing from a zero PSW. Machine-check, system/external, and timer
interrupts await the I/O phases; program interrupts leave the IC per
Figure 2-20's "PSW can vary" note (illegal: at the instruction;
others: past it).

| Instr | Status | Notes |
|---|---|---|
| LPS | VERIFIED | privileged; loads both PSW words; CC/indicators from new PSW (§9.3) |
| SPM | VERIFIED | R2 bits 16-23 → CC/carry/overflow/masks (§9.5) |
| SSM | VERIFIED | privileged; halfword → PSW bits 32-47 incl. wait/problem/register-set (§9.6) |
| SVC | VERIFIED | §9.9 |
| TS | VERIFIED | three-state CC then set all ones; atomic (trivially — no concurrent bus masters yet) (§9.10) |

## Special operations (§9) — implemented (phase 2)

| Instr | Status | Notes |
|---|---|---|
| ISPB | VERIFIED | privileged; M1 selects set/reset × halfword/fullword; M1 1xx illegal; halfword index alignment (§9.2) |
| MVH | PARTIAL | §9.4 block move with DSR/DSE sector selection, high-address-first, count left in R1; store-protect violation backs the IC up with the remaining count. Executed atomically (no async interrupts yet); the §9.4 ANOMALY (source = destination+1 with differing MSBs) is not replicated |
| SCAL / SRET | VERIFIED | 18-halfword stack frame (PSW word 0 + 8 GPRs), SSD update PTR+=INC/INC=18, conditional restore (§9.7/9.8) |
| TSB | VERIFIED | three-state test then OR, atomic (§9.11) |
| LXAR/LXA, STXAR/STXA | VERIFIED | fullword address-constant load/store of R1 bits 1-15 + its DSE; STXA keeps destination bits 20-27 (§9.12/9.14) |
| LDM, STDM | VERIFIED | R0-R3 DSEs as packed nibbles (§9.13/9.15); only R0-R3 — R4-R7 DSEs are not covered, as §9.4's notes warn |

## Storage protection and instruction monitor (§2.4) — implemented (phase 2)

Every CPU store checks the per-halfword protection bit: a violation takes
the unmaskable program interrupt (code 0007) and the store does not
occur. The instruction monitor (PSW bit 34) interrupts through 0070/0074
on executing an unprotected instruction word, IC left at the offender
(AP-101S behavior, §2.4.1). The Figure 2-20 anomaly notes about CC=10 in
the old PSW for these interrupts are not replicated.

## I/O operations

| Instr | Status | Notes |
|---|---|---|
| PC | PARTIAL | §3.3 implemented at the CPU boundary: privileged, CW in R2 (bit 0 input/output), data in R1, CC 00 success / 01 timeout; the subsystem side is a pluggable `IoSubsystem` trait (no subsystem attached = timeout). Appendix I command semantics await the IOP model. **Source conflict:** yaGPC2/nsts-sim-gpc take the CW from R1 and the data from R2; the PoO text says the opposite and is followed here |
| ICR, DIAG | NOT IMPLEMENTED (encodings verified) | internal-control timers and diagnose need the IOP/hardware model; decode and trap |

## Addressing-mode gap (PARTIAL)

One mode remains unimplemented: the **fullword indirect address pointer
with postindexing** (RS indexed, X≠0/IA=1/I=1 — §2.2.8 step 10, Figure
2-17, with its Xc/C/CB/CD/BSV/DSV control bits). It traps as
`UnimplementedAddressing`; the Figure 2-17 flowchart page needs a
higher-quality read before implementation. Everything else in the §11.1
chart is implemented and tested, including the X=0 fullword indirect
with automatic storage modification (Figure 2-15).

## Interrupts (other classes), protection, timing — NOT IMPLEMENTED

- Program-exception and SVC interrupts are modeled (above). Machine
  check, system/external, and interval-timer interrupts are not (I/O
  phases).
- Storage protection and the instruction monitor are modeled (above).
- Timing (§16-§17) is not modeled; this is an instruction-level emulator.

## UNVERIFIED / open questions

- **XUL semantics under disagreement** — see the fixed-point table.
- **BCR/BCTR with R2=0**: the manual gives a "no branch" note only for
  BALR (§5.1). Following the text (and yaGPC2), BCR/BCTR branch to the
  address in GR0. No flight-code evidence either way.
- **AP-101B behavior**: everything here is AP-101S. Known AP-101B
  differences (even-boundary alignment §2.1.3, instruction-monitor IC
  behavior §2.4.1, memory size) are documented but nothing AP-101B-
  specific is implemented. IBM 6246156B / 75-A97-001 are the sources to
  read for a future AP-101B mode.
- Whether a computed shift count of zero also behaves as "no operation"
  (assumed here) versus a 0-bit shift with carry effects: the manual's
  no-op wording is attached to the count *field* being zero (Figure 6-1);
  the two readings are indistinguishable in this implementation because a
  0-count shift changes nothing and leaves carry alone.
