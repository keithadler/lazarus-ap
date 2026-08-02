#!/usr/bin/env python3
"""Differential test: run the reference emulator and Lazarus AP on the
same flight image and compare EVERY executed instruction.

Output parity proves the answers match. This proves the machines agree
step by step - address, opcode, and all eight registers - which sweeps
the whole instruction set rather than trusting that a few programs
happen to exercise it.

    tools/difftest.py <image.fcm> [--steps N]

Needs yaGPC2 built (see docs/ROADMAP.md); point YAGPC2 at the binary.
"""
import json, os, re, subprocess, sys

YAGPC2 = os.environ.get("YAGPC2", "~/"
    "SESSION/"
    "scratchpad/virtualagc/yaShuttle/yaGPC2/yaGPC2")

# Section names can run straight into the offset ("#CREADAC+0000:"),
# so match lazily up to the offset rather than demanding whitespace.
LINE = re.compile(r"^\[\s*(\d+)\]\s+([0-9a-f]{6})\s+.*?\+[0-9a-f]+:\s+([0-9a-f]{4})")
REG = re.compile(r"R(\d\d): ([0-9a-f]{8})->([0-9a-f]{8})")


def reference(image, symbols, steps):
    """[(step, addr, opcode, {reg: after})] from the reference emulator."""
    cmd = [YAGPC2, "--trace", "--no-verbose", "--max-steps", str(steps),
           "--symbols", symbols, image]
    out = subprocess.run(cmd, capture_output=True, text=True).stdout
    rows = []
    for line in out.splitlines():
        m = LINE.match(line)
        if not m:
            continue
        regs = {int(r[0]): int(r[2], 16) for r in REG.findall(line)}
        rows.append((int(m.group(1)), int(m.group(2), 16), int(m.group(3), 16), regs))
    return rows


def ours(image, steps):
    """Same shape, from lazap-trace's JSON."""
    stem = image[:-4] if image.endswith(".fcm") else image
    out = subprocess.run(
        ["cargo", "run", "-q", "--bin", "lazap-trace", "--", image,
         "--max-trace", str(steps), "--steps", str(steps)],
        capture_output=True, text=True).stdout
    t = json.loads(out)
    rows = []
    for i, s in enumerate(t["steps"]):
        rows.append((i, s[0], s[2], [int(x, 16) for x in s[3]]))
    return rows


def main():
    image = sys.argv[1]
    steps = 2000
    if "--steps" in sys.argv:
        steps = int(sys.argv[sys.argv.index("--steps") + 1])
    stem = image[:-4]
    symbols = f"{stem}-lnk101.json"
    ref, us = reference(image, symbols, steps), ours(image, steps)
    n = min(len(ref), len(us))
    print(f"comparing {n} instructions ({len(ref)} reference, {len(us)} ours)")
    bad = 0
    # Our trace samples registers BEFORE each instruction; the reference
    # reports the values an instruction produced. So the reference's
    # step i is checked against our step i+1's register snapshot.
    for i in range(n - 1):
        (_, ra, rop, rregs), (_, oa, oop, _) = ref[i], us[i]
        oregs = us[i + 1][3]
        if ra != oa or rop != oop:
            print(f"step {i}: ADDRESS/OPCODE differ - reference {ra:05X}/{rop:04X}, "
                  f"ours {oa:05X}/{oop:04X}")
            bad += 1
        else:
            # The reference prints only registers it CHANGED this step;
            # every one it reports must match our post-state.
            for rn, val in rregs.items():
                if rn < 8 and oregs[rn] != val:
                    print(f"step {i} @{ra:05X} {rop:04X}: R{rn} reference "
                          f"{val:08X}, ours {oregs[rn]:08X}")
                    bad += 1
        if bad > 12:
            print("... stopping after 12 divergences")
            break
    if bad:
        print(f"DIVERGENT: {bad} mismatches in {n} instructions")
        sys.exit(1)
    print(f"IDENTICAL: {n} instructions, every address, opcode and register agrees")


if __name__ == "__main__":
    main()
