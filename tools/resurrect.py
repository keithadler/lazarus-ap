#!/usr/bin/env python3
"""Resurrect a genuine Shuttle flight routine: generate a driver, walk
its #Q dependency closure, assemble with the real ASM101S, link with
lnk_lite, run on Lazarus AP, and compare against modern math.

    tools/resurrect.py SQRT:2.0 EXP:1.0 LOG:2.0 ...

Each argument is ROUTINE:ARG (ARG in decimal). Emits a markdown row per
routine and exits non-zero if any answer is wrong.
"""
import math, os, struct, subprocess, sys
sys.path.insert(0, os.path.dirname(__file__))
from lnk_lite import parse_deck

VAGC = os.environ.get("VAGC", "~/"
    ""
    "virtualagc")
ASM = f"{VAGC}/ASM101S"
PY = os.environ.get("VENV_PY", f"{os.path.dirname(VAGC)}/venv/bin/python3")
RUNASM = f"{VAGC}/yaShuttle/Source Code/PASS.REL32V0/RUNASM"
OBJ = os.environ.get("CENSUS_DIR", "/tmp")   # census_<NAME>.obj lives here
OUT = "roms/nasa"

EXPECT = {  # modern equivalents
    "SQRT": math.sqrt, "EXP": math.exp, "LOG": math.log, "TAN": math.tan,
    "SINH": math.sinh, "TANH": math.tanh, "COSH": math.cosh,
    "ACOS": math.acos, "ASINH": math.asinh, "ATANH": math.atanh,
    "ACOSH": math.acosh, "ATAN": math.atan,
    "EATAN2": math.atan2, "EMOD": math.fmod, "EPWRE": math.pow,
    "DSQRT": math.sqrt, "DEXP": math.exp, "DLOG": math.log,
    "DTAN": math.tan, "DSINH": math.sinh, "DTANH": math.tanh,
    "DACOS": math.acos, "DATANH": math.atanh, "DMOD": math.fmod,
}


def ibm_hex(v):
    """Python float -> IBM short hexfloat word (truncating)."""
    if v == 0:
        return 0
    neg, av, ch = v < 0, abs(v), 64
    while av >= 1.0:
        av /= 16.0; ch += 1
    while av < 1 / 16.0:
        av *= 16.0; ch -= 1
    frac = int(av * (1 << 24))
    return (neg << 31) | ((ch & 0x7F) << 24) | (frac & 0xFFFFFF)


def ibm_hex_long(v):
    """Python float -> IBM long hexfloat (hi, lo) words, truncating."""
    if v == 0:
        return 0, 0
    neg, av, ch = v < 0, abs(v), 64
    while av >= 1.0:
        av /= 16.0; ch += 1
    while av < 1 / 16.0:
        av *= 16.0; ch -= 1
    frac = int(av * (1 << 56))
    return ((neg << 31) | ((ch & 0x7F) << 24) | ((frac >> 32) & 0xFFFFFF),
            frac & 0xFFFFFFFF)


def deps(obj):
    return {e["name"][2:] for m in parse_deck(obj) for e in m["esds"]
            if e["kind"] == "ER" and e["name"].startswith("#Q")}


def census_obj(name):
    """Deck defining `name` — which may be an alternate entry inside
    another routine's source (COSH lives in SINH.asm, SIN in SNCS.asm)."""
    p = f"{OBJ}/census_{name}.obj"
    if os.path.exists(p):
        return p
    if os.path.exists(f"{RUNASM}/{name}.asm"):
        subprocess.run([PY, "ASM101S.py", "--library", f"--object={p}",
                        f"{RUNASM}/{name}.asm"], cwd=ASM, capture_output=True)
        if os.path.exists(p):
            return p
    import glob
    for cand in sorted(glob.glob(f"{OBJ}/census_*.obj")):
        try:
            for m in parse_deck(cand):
                if any(e["kind"] == "SD" and e["name"] == name
                       for e in m["esds"]):
                    return cand
        except Exception:
            pass
    raise SystemExit(f"no deck defines {name}")


def resurrect(name, arg, arg2=None):
    chain, todo = [], [name]
    while todo:
        n = todo.pop()
        if n in chain:
            continue
        chain.append(n)
        todo += [d for d in deps(census_obj(n)) if d not in chain]
    # Intrinsics (AMAIN INTSIC=YES) are entered by a plain BAL 4 with a
    # frame in R0; LIB routines go through the ACALL stub sequence.
    try:
        srctext = open(f"{RUNASM}/{name}.asm", errors="ignore").read()
    except OSError:
        srctext = ""
    intrinsic = "INTSIC=YES" in srctext
    dp = "SCALAR DP" in srctext.split("OUTPUT", 1)[-1][:60]
    # A second scalar argument travels in F2 (the routines' own INPUT
    # lines: "INPUT F0, ... / F2 ...").
    ld2 = "         LE    2,ARG2\n" if arg2 is not None else ""
    if dp:
        hi, lo = ibm_hex_long(arg)
        argdc = f"ARG      DC    X'{hi:08X}',X'{lo:08X}'\n"
        loadc = "         LED   0,ARG\n"
        stc = "         STED  0,RESULT\n"
        resdc = "RESULT   DC    2F'0'\n"
    else:
        argdc = f"ARG      DC    X'{ibm_hex(arg):08X}'\n"
        loadc = "         LE    0,ARG\n"
        stc = "         STE   0,RESULT\n"
        resdc = "RESULT   DC    F'0'\n"
    if arg2 is not None:
        argdc += f"ARG2     DC    X'{ibm_hex(arg2):08X}'\n"
    if intrinsic:
        src = (f"         EXTRN {name}\nDRIVER   CSECT\n"
               "         LA    0,STK\n" + loadc + ld2 +
               f"         BAL   4,{name}\n" + stc +
               "         SVC   ENDC\n"
               "ENDC     DC    H'21'\n" + argdc + resdc +
               "STK      DS    40F\n         END   DRIVER\n")
        open(f"{ASM}/{name}_DRV.asm", "w").write(src)
        lst = subprocess.run([PY, "ASM101S.py", f"--object={name}_DRV.obj",
                             f"{name}_DRV.asm"], cwd=ASM,
                             capture_output=True, text=True)
        return _finish(name, chain, dp, result_at(lst.stdout))
    src = (f"         EXTRN #Q{name}\nDRIVER   CSECT\n"
           "         LA    0,STK\n" + loadc + ld2 +
           "         DC    X'D0FF'\n"
           f"         DC    Y(#Q{name}+14336)\n" + stc +
           "         SVC   ENDC\n"
           "ENDC     DC    H'21'\n" + argdc + resdc +
           "STK      DS    40F\n         END   DRIVER\n")
    open(f"{ASM}/{name}_DRV.asm", "w").write(src)
    lst = subprocess.run([PY, "ASM101S.py", f"--object={name}_DRV.obj",
                          f"{name}_DRV.asm"], cwd=ASM,
                          capture_output=True, text=True)
    return _finish(name, chain, dp, result_at(lst.stdout))


def result_at(listing):
    """RESULT's halfword offset within DRIVER, from ASM101S's own symbol
    table (drivers differ in length, so never assume)."""
    for line in listing.splitlines():
        f = line.split()
        if len(f) >= 3 and f[0] == "RESULT":
            return 0x100 + int(f[2], 16)
    return 0x10E


def _finish(name, chain, dp=False, at=0x10E):
    if not os.path.exists(f"{ASM}/{name}_DRV.obj"):
        return name, chain, None, "driver assembly failed"
    with open(f"{OUT}/{name}RUN.obj", "wb") as f:
        f.write(open(f"{ASM}/{name}_DRV.obj", "rb").read())
        for n in chain:
            f.write(open(census_obj(n), "rb").read())
    r = subprocess.run(["python3", "tools/lnk_lite.py", f"{OUT}/{name}RUN.obj",
                        "--standalone", "-o", f"{OUT}/{name}RUN.fcm"],
                       capture_output=True, text=True)
    if r.returncode:
        return name, chain, None, r.stderr.strip()[:60]
    cmd = ["cargo", "run", "-q", "--bin", "lazap-call", "--",
           f"{OUT}/{name}RUN.fcm", "--at", f"{at:X}"] + (["--dp"] if dp else [])
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode:
        return name, chain, None, (r.stdout + r.stderr).strip()[:60]
    _, word, val = r.stdout.split()
    return name, chain, (word, float(val)), None


def main():
    rows, bad = [], 0
    for spec in sys.argv[1:]:
        name, _, a = spec.partition(":")
        args = [float(x) for x in a.split(",")]
        arg = args[0]
        arg2 = args[1] if len(args) > 1 else None
        name, chain, got, err = resurrect(name, arg, arg2)
        if err:
            rows.append(f"| {name} | {'&rarr;'.join(chain[1:]) or '&mdash;'} "
                        f"| {arg:g}{'' if arg2 is None else f', {arg2:g}'} "
                        f"| &mdash; | &mdash; | FAILED: {err} |")
            bad += 1
            continue
        word, val = got
        want = EXPECT[name](arg, arg2) if arg2 is not None else EXPECT[name](arg)
        ok = abs(val - want) < 2e-6 * max(1, abs(want))
        bad += not ok
        rows.append(f"| {name} | {'&rarr;'.join(chain[1:]) or '&mdash;'} "
                    f"| {arg:g}{'' if arg2 is None else f', {arg2:g}'} "
                    f"| 0x{word} = {val:.7f} | {want:.7f} "
                    f"| {'OK' if ok else 'MISMATCH'} |")
        print(rows[-1])
    open(f"{OUT}/lab.md", "w").write(
        "| Routine | Calls | Input | Flight answer | Modern | Status |\n"
        "|---|---|---|---|---|---|\n" + "\n".join(rows) + "\n")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
