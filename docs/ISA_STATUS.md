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

## Floating point (§8) — NOT IMPLEMENTED (encodings verified)

The format is documented and VERIFIED (IBM hexadecimal float, sign +
7-bit excess-64 characteristic + hex fraction; short = 24-bit fraction,
long = 56-bit in a register pair; CC rules per §8.7). Execution is out of
phase-1 scope; all encodings below decode and trap with their mnemonic:

AER/AE/AEDR/AED (add), SER/SE/SEDR/SED (subtract), CER/CE/CEDR/CED
(compare), MER/ME/MEDR/MED (multiply), DER/DE/DEDR/DED (divide),
LER/LE/LED/LECR (load), STE/STED (store), CVFX/CVFL (convert),
MVS (midvalue select), LFLI/LFLR/LFXR (register moves/immediates).

## Special, status-switching, and I/O operations (§3, §9, §10) — NOT IMPLEMENTED (encodings verified)

DIAG, ISPB, LPS, MVH, SPM, SSM, SCAL, SRET, SVC, TS, TSB, LXA/LXAR,
LDM, STXA/STXAR, STDM, ICR, PC. These require the interrupt/storage-
protection/DSE/IOP machinery of later phases. All decode and trap.

## Addressing-mode gap (PARTIAL, applies to all RS-indexed forms)

The **fullword indirect address pointer** modes (RS indexed with IA=1 and
I=1, and X=0/IA=0 variants thereof — §2.2.8 steps 7 and 10, Figure 2-17,
including automatic storage modification and the BSV/DSV/PSW-modify
control bits) are decoded but trap as `UnimplementedAddressing`. All
other §11.1 modes (SRS, RS extended, indexed, IC-relative ±, halfword
indirect, postindexed indirect, automatic index modification) are
implemented and tested.

## Interrupts, protection, timing — NOT IMPLEMENTED

- The interrupt system (§2.5.2) is not modeled. The one interrupt whose
  trigger arises in phase-1 code — fixed-point overflow with PSW bit 20
  set — halts the emulator with a typed trap instead of PSW-swapping.
- Storage protection and the instruction monitor (§2.4) are not modeled.
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
