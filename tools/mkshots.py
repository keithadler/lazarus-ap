#!/usr/bin/env python3
"""Render the README's terminal cards as SVG, from real program output.

Nothing here is a mockup. Each card runs the actual binary and typesets
whatever it prints, so the images cannot drift from what the code does
(and CI regenerates them to check they haven't).

Usage: python3 tools/mkshots.py [--check]
"""
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "docs/img"
BIN = ROOT / "target/release"

# A terminal card that reads well on both GitHub themes: it carries its
# own dark ground rather than relying on the page's.
BG, FG, DIM, ACC, WARN = "#12151A", "#D6DBE1", "#7E8894", "#7FA8E8", "#E8B45C"
CH_W, LINE_H, PAD_X, PAD_Y, TITLE_H = 8.05, 20, 20, 16, 40

ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")


def esc(t):
    return t.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def card(title, lines, highlight=(), accent=()):
    """highlight: substrings drawn in amber. accent: whole-line indices."""
    lines = [ln.rstrip() for ln in lines]
    w = max([len(l) for l in lines] + [len(title) + 6])
    width = int(w * CH_W + PAD_X * 2)
    height = int(TITLE_H + len(lines) * LINE_H + PAD_Y)
    rows = []
    for i, ln in enumerate(lines):
        y = TITLE_H + PAD_Y + i * LINE_H - 4
        fill = ACC if i in accent else FG
        if not ln.strip():
            continue
        body = esc(ln)
        for h in highlight:
            if h in ln:
                body = body.replace(esc(h), f'</tspan><tspan fill="{WARN}">{esc(h)}'
                                            f'</tspan><tspan fill="{fill}">')
        # xml:space keeps the column alignment the programs actually
        # print; without it SVG collapses the leading runs of spaces.
        rows.append(f'<text xml:space="preserve" x="{PAD_X}" y="{y}" '
                    f'fill="{fill}"><tspan>{body}</tspan></text>')
    dots = "".join(
        f'<circle cx="{PAD_X + 6 + i * 16}" cy="20" r="5" fill="{c}"/>'
        for i, c in enumerate(("#E06C60", "#E8B45C", "#8FBF6E")))
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}"
     viewBox="0 0 {width} {height}" font-family="ui-monospace,SFMono-Regular,Menlo,monospace"
     font-size="13" role="img" aria-label="{esc(title)}">
  <rect width="{width}" height="{height}" rx="8" fill="{BG}"/>
  <rect width="{width}" height="{TITLE_H}" rx="8" fill="#1A1F26"/>
  <rect y="{TITLE_H - 8}" width="{width}" height="8" fill="#1A1F26"/>
  {dots}
  <text x="{PAD_X + 62}" y="25" fill="{DIM}" font-size="12">{esc(title)}</text>
  {chr(10).join("  " + r for r in rows)}
</svg>
'''


def run(args, stdin=subprocess.DEVNULL):
    p = subprocess.run([str(BIN / args[0])] + args[1:], capture_output=True,
                       text=True, stdin=stdin, cwd=ROOT)
    return ANSI.sub("", p.stdout)


def main():
    check = "--check" in sys.argv
    if not (BIN / "lazap").exists():
        sys.exit("build first: cargo build --release --bins")
    OUT.mkdir(parents=True, exist_ok=True)
    shots = {}

    # 1. A real HAL/S program, compiled by the Shuttle's own compiler.
    out = run(["lazap", "roms/hello/hello.fcm"]).splitlines()
    shots["hello.svg"] = card(
        "cargo run --bin lazap -- roms/hello/hello.fcm", out[:9])

    # 2. What it cost.
    out = run(["lazap-time", "roms/hello/hello.fcm", "--assume-microseconds"])
    shots["timing.svg"] = card(
        "cargo run --bin lazap-time -- roms/hello/hello.fcm --assume-microseconds",
        out.splitlines(), highlight=("44",), accent=(0,))

    # 3. Five computers, one of them lying.
    out = run(["lazap-set", "--fault", "2", "--fast"]).splitlines()
    keep = [l for l in out if l.strip()][:10]
    shots["vote.svg"] = card(
        "cargo run --bin lazap-set -- --fault 2",
        keep, highlight=("13", "BYPASSED"))

    # 4. The flight mathematics, re-checked.
    p = subprocess.run([sys.executable, "tools/labcheck.py"],
                       capture_output=True, text=True, cwd=ROOT)
    tbl = (ROOT / "roms/nasa/lab.md").read_text().splitlines()
    rows = [l for l in tbl if l.startswith("|")][2:]
    def cell(r, i):
        return [c.strip() for c in r.strip("|").split("|")][i]
    body = ["  routine      input            flight answer      modern", ""]
    for r in rows:
        if cell(r, 0).strip("`") in ("SQRT", "SNCS", "VV6", "VV10", "MM14", "DPROD"):
            body.append(f"  {cell(r,0).strip('`'):<12} {cell(r,2):<16} "
                        f"{cell(r,3):<18} {cell(r,4)}")
    body += ["", p.stdout.strip()]
    shots["flightmath.svg"] = card("python3 tools/labcheck.py", body,
                                   highlight=("39/39",))

    stale = []
    for name, svg in shots.items():
        f = OUT / name
        if check and (not f.exists() or f.read_text() != svg):
            stale.append(name)
        else:
            f.write_text(svg)
    if check and stale:
        sys.exit(f"stale README images (run tools/mkshots.py): {stale}")
    print(f"{len(shots)} cards -> {OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
