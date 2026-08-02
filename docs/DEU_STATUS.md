# Crew interface (DEU) status — phase 5

## Sourced

- Architecture: DEUs sit on display buses as polled bus terminal
  units; keystrokes travel to the GPCs and display data travels back as
  ordinary serial-bus transactions (NASA DPS Overview Workbook; the
  Virtual AGC library hosts it).
- The 32-key DPS keyboard key set (hex 0-9/A-F, +, -, ., OPS, SPEC,
  ITEM, EXEC, PRO, RESUME, CLEAR, SYS SUMM, FAULT SUMM, GPC/CRT,
  I/O RESET, ACK — Shuttle Crew Operations Manual).
- The GPC side of every transaction: real BCE #MIN (poll) and #MOUT
  (display write) instructions per App. III, including the §3.4.3
  listen-mode first-input-command rule (added for the eavesdrop case).

## EMULATOR CONVENTION (not from a primary source)

The DEU wire protocol — command opcodes (1 = poll keystrokes, 2 =
display write with cursor+count), keystroke word encoding (one code
per data word, 0xFF = buffer empty), and key codes — is ours. The real
DEU protocol has not surfaced in the recovered documents; the model is
built to be swapped out if it does. The screen is a plain rows x cols
text buffer (real: 26 x 51 monochrome CRT with format overlays managed
by DEU firmware).

## Working today (tests/crew_interface.rs)

A GPC polls "OPS 2 0 1 PRO" off a DEU keyboard into main storage and
paints "OPS 201" onto the CRT, both via BCE programs over a display
bus; a BFS-style listener GPC on the same bus overhears every
keystroke — how the real BFS tracked crew inputs to PASS.

## Front end

`cargo run --bin lazap-dps` opens the interactive crew station: your
keystrokes queue in the DEU, the GPC's #MIN poll loop pulls them over
the display bus into main storage (the "GPC HEARD" line reads GPC
memory, not the DEU), and the CRT header was painted by the GPC's
#MOUT. `--demo` types "OPS 2 0 1 PRO" itself for a non-interactive
smoke test.

## Staged

- DEU format overlays / decoms (needs the DPS Dictionary formats).
- CPU-side keyboard-echo software driven by IOP interrupts rather than
  host pumping (needs LOAD LOCAL STORE so the CPU can task the MSC).
