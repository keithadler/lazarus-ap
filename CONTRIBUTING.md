# Contributing

This project reconstructs a machine nobody can test against any more.
That single fact sets every rule below.

## The standard: never guess

The AP-101S instruction set is not fully documented in public sources.
The temptation, constantly, is to fill a gap with something plausible.
Don't. A plausible guess that runs is worse than an honest hole,
because it looks like knowledge.

Concretely:

- **Cite what you implement.** Every instruction carries its page in
  IBM 85-C67-001. Every behaviour traceable to a document says which.
- **Mark what you don't know `UNVERIFIED`**, in the code comment *and*
  in [docs/ISA_STATUS.md](docs/ISA_STATUS.md). Leave it unimplemented
  rather than approximated.
- **Don't launder a secondary source into a primary one.** If a number
  came from another emulator or a compiler table rather than the
  hardware manual, say so where it lands. `src/timing.rs` is the model:
  it names its origin, names the intermediate port, and flags that the
  *unit* is still unconfirmed.
- **State the boundary when evidence runs out.** 176-P's object deck
  did not survive, so the resource page declines to say which routines
  were its own. "We don't know" is a publishable result.

If you find behaviour this emulator gets wrong against period
documentation or hardware evidence, that is a bug — please report it
with the source.

## Claims must be generated, not maintained by hand

Three separate files once each claimed a different number of working
routines, and all three were wrong. Two committed artifacts once
asserted things that were false while every test passed
([DEFECTS.md](roms/nasa/DEFECTS.md), LAZARUS-2 and LAZARUS-3).

So: anything the project asserts about itself should be produced by a
tool and checked in CI.

- `tools/labcheck.py` runs every image in `roms/nasa/lab.json` against
  modern mathematics and regenerates `lab.md`. CI fails if they drift.
- `tools/check_symtabs.py` fails if a linked image carries another
  program's symbol table.

Adding a routine means adding it to the manifest, not editing a table.

## Our own bugs go in the defect report

`roms/nasa/DEFECTS.md` leads with defects in the flight system and its
documentation, and carries ours in an appendix. A resurrection that
lists only other people's bugs is not being honest. Green tests are not
evidence of correctness — both of our recorded defects passed every
test in the suite.

## Working on it

```bash
cargo test                                  # the full suite
python3 tools/labcheck.py                   # every routine vs modern maths
python3 tools/check_symtabs.py              # artifact provenance
cargo fmt && cargo clippy
```

Note that `lazap` reads standard input, so redirect it (`< /dev/null`)
when scripting a run that expects no input, or it will wait forever.

Commit messages here explain *why*, and say plainly what was wrong when
something was wrong. Please match that.

## Provenance of the artifacts

The recovered material — the AP-101S manuals, the HAL/S compiler
source, ASM101S, the runtime library, the fixtures — exists because of
**Ron Burkey's Virtual AGC project** and **Don Schmidt's** AP-101S
work. Anything derived from them should say so. The NASA/IBM sources
are public domain; the Virtual AGC artifacts are GPL v2-or-later, which
is why this project is too. [NOTICE](NOTICE) has the full breakdown —
please keep it accurate when you add anything derived from elsewhere.
