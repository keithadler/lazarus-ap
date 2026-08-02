# Lazarus AP

A faithful, test-driven emulator of the IBM AP-101 — the general-purpose
computer (GPC) that flew as the Space Shuttle's flight computer.

**Phase 1** (this repository's current state): CPU + memory emulator for the
AP-101S "Shuttle instruction set", built strictly from primary sources, with
an instruction-level test suite. No HAL/S compiler, no flight software yet.

See:

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — design and language choice
- [docs/ISA_STATUS.md](docs/ISA_STATUS.md) — per-instruction implementation and
  verification status (what is VERIFIED against primary sources vs UNVERIFIED)
- [docs/SOURCES.md](docs/SOURCES.md) — the primary sources used and what each
  one confirmed
- [docs/PRIOR_ART.md](docs/PRIOR_ART.md) — existing Shuttle GPC software
  reconstruction efforts and how Lazarus AP relates to them

## Build and test

Requires a Rust toolchain (stable).

```
cargo build
cargo test
```

Both commands are run from the repository root. `cargo test` runs the full
instruction-level suite plus golden-trace integration tests and reports
pass/fail per test.
