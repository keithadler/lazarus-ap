#!/usr/bin/env python3
"""Build docs/index.html (the GitHub Pages site) from docs/walkthrough.html.

walkthrough.html is authored as a document body — it is also published
as a hosted artifact, where the host supplies the surrounding document.
Served directly from GitHub Pages there is no host, so this adds the
wrapper the page needs to work as a real web page: a charset (without
which the typography mojibakes), a viewport for phones, and the
Open Graph / Twitter card metadata that makes a shared link render as a
preview instead of a bare URL.

Also renders the social preview image. Requires macOS qlmanage + sips;
skips with a warning elsewhere, since the committed PNG is what ships.

Usage: python3 tools/mkpage.py [--check]
"""
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / "docs/walkthrough.html"
OUT = ROOT / "docs/index.html"
OG = ROOT / "docs/img/social.png"

SITE = "https://keithadler.github.io/lazarus-ap/"
TITLE = "Watching a Space Shuttle Computer Think"
DESC = ("The IBM AP-101S — the computer that flew every Shuttle mission — "
        "emulated from IBM's own manuals and running real NASA flight code. "
        "Step through it instruction by instruction.")

# The artifact copy links to the hosted live console; the Pages copy
# links to its neighbour.
ARTIFACT_LIVE = "https://claude.ai/code/artifact/a57abad3-1398-4d1e-9676-e9f7e398e97d"


def social_svg():
    """Authored on a 1200x1200 square with the card centred in the middle
    630 rows. qlmanage renders SVG into a square canvas, so laying it out
    square keeps the render 1:1 and lets sips crop the exact card back
    out without scaling or clipping."""
    T = 285  # top of the card band within the square
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="1200"
     viewBox="0 0 1200 1200" font-family="Helvetica,Arial,sans-serif">
  <rect width="1200" height="1200" fill="#0B0F13"/>
  <rect x="0" y="{T}" width="1200" height="6" fill="#0B3D91"/>
  <text x="70" y="{T + 112}" fill="#7E8A93" font-size="20" letter-spacing="3.5"
        font-weight="600">IBM AP-101S &#183; GENERAL PURPOSE COMPUTER</text>
  <text x="70" y="{T + 216}" fill="#F2F5F8" font-size="66" font-weight="700">Watching a Space Shuttle</text>
  <text x="70" y="{T + 292}" fill="#F2F5F8" font-size="66" font-weight="700">Computer Think</text>
  <text x="70" y="{T + 364}" fill="#A8B2BC" font-size="25">Emulated from IBM&#8217;s own manuals. Running real NASA flight code.</text>
  <rect x="70" y="{T + 418}" width="1060" height="1" fill="#28313A"/>
  <text x="70" y="{T + 486}" fill="#5BE07E" font-size="24"
        font-family="ui-monospace,Menlo,monospace">SQRT(2) = 1.4142132</text>
  <text x="70" y="{T + 530}" fill="#7E8A93" font-size="19"
        font-family="ui-monospace,Menlo,monospace">the Shuttle&#8217;s own answer, first run since Atlantis</text>
  <text x="1130" y="{T + 530}" fill="#F0A93B" font-size="19" text-anchor="end"
        font-family="ui-monospace,Menlo,monospace">40 routines &#183; 194 tests</text>
</svg>
'''


def build_page():
    body = SRC.read_text()
    body = body.replace(f'<a class="livebar" href="{ARTIFACT_LIVE}">',
                        '<a class="livebar" href="live.html">')
    # The body opens with its own <title>; the wrapper owns that instead.
    body = re.sub(r"^<title>.*?</title>\s*", "", body, count=1, flags=re.S)
    return f'''<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{TITLE}</title>
<meta name="description" content="{DESC}">
<link rel="canonical" href="{SITE}">

<meta property="og:type" content="website">
<meta property="og:url" content="{SITE}">
<meta property="og:title" content="{TITLE}">
<meta property="og:description" content="{DESC}">
<meta property="og:image" content="{SITE}img/social.png">
<meta property="og:image:width" content="1200">
<meta property="og:image:height" content="630">
<meta property="og:site_name" content="Lazarus AP">

<meta name="twitter:card" content="summary_large_image">
<meta name="twitter:title" content="{TITLE}">
<meta name="twitter:description" content="{DESC}">
<meta name="twitter:image" content="{SITE}img/social.png">

<meta name="theme-color" content="#0B3D91">
<link rel="icon" href="data:image/svg+xml,<svg xmlns=%22http://www.w3.org/2000/svg%22 viewBox=%220 0 16 16%22><text y=%2214%22 font-size=%2214%22>&#128640;</text></svg>">
</head>
<body>
{body}
</body>
</html>
'''


def build_social():
    if not shutil.which("qlmanage") or not shutil.which("sips"):
        print("  (skipping social.png: needs macOS qlmanage + sips)")
        return
    with tempfile.TemporaryDirectory() as td:
        svg = pathlib.Path(td) / "social.svg"
        svg.write_text(social_svg())
        subprocess.run(["qlmanage", "-t", "-s", "1200", "-o", td, str(svg)],
                       capture_output=True)
        png = pathlib.Path(td) / "social.svg.png"
        if not png.exists():
            print("  (social.png render failed; keeping committed copy)")
            return
        # qlmanage pads to a square; crop back to the 1200x630 card.
        subprocess.run(["sips", "-c", "630", "1200", str(png), "--out", str(OG)],
                       capture_output=True)
        print(f"  social.png {OG.stat().st_size // 1024} KB")


def main():
    check = "--check" in sys.argv
    page = build_page()
    if check:
        if not OUT.exists() or OUT.read_text() != page:
            sys.exit("docs/index.html is stale — run: python3 tools/mkpage.py")
        print("docs/index.html is current")
        return
    OUT.write_text(page)
    print(f"docs/index.html {OUT.stat().st_size // 1024} KB")
    build_social()


if __name__ == "__main__":
    main()
