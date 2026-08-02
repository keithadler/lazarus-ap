# Publishing this page

`docs/index.html` is the whole resource: one self-contained file, no
build step, no external requests. GitHub Pages serves it directly —
Settings → Pages → Source: `main` branch, `/docs` folder.

Regenerate its embedded data after changing the emulator:

    cargo run --bin lazap-trace -- roms/lazarus/LAZARUS.fcm > trace.json
    cargo run --bin lazap-set -- --trace > vote.json
    cargo run --bin lazap-dps -- --trace > deu.json
    tools/resurrect.py SQRT:2.0 EXP:1.0 ...      # regenerates roms/nasa/lab.md
