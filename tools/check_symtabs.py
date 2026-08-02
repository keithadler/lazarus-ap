#!/usr/bin/env python3
"""Guard against a linked image carrying another program's symbol table.

LAZARUS shipped for a while with a byte-identical copy of the hello
fixture's table (see DEFECTS.md LAZARUS-2), which silently misattributed
every address in the walkthrough.  Two checks catch a recurrence:

  1. No two non-standalone tables may be byte-identical.
  2. A table's own CSECT names must appear in the matching .obj deck.

Standalone links (tools/lnk_lite.py --standalone) emit an intentionally
empty stub table, so they are exempt from the first check.
"""
import json, pathlib, sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from lnk_lite import parse_deck

root = pathlib.Path(__file__).resolve().parent.parent
tables, seen, bad = [], {}, []

for p in sorted(root.glob("roms/**/*-lnk101.json")):
    d = json.loads(p.read_text())
    if not d.get("sections"):
        continue  # standalone stub
    tables.append((p, d))
    key = json.dumps(d, sort_keys=True)
    if key in seen:
        bad.append(f"{p.relative_to(root)} is byte-identical to "
                   f"{seen[key].relative_to(root)}")
    seen[key] = p

for p, d in tables:
    obj = p.parent / (p.name[:-len("-lnk101.json")] + ".obj")
    if not obj.exists():
        continue
    declared = {e["name"] for m in parse_deck(obj) for e in m["esds"]}
    owned = {s["name"] for s in d["sections"]
             if s["name"][:2] in ("$0", "#D", "#E", "@0")}
    missing = sorted(owned - declared)
    if missing:
        bad.append(f"{p.relative_to(root)} claims CSECTs absent from "
                   f"{obj.name}: {missing}")

for line in bad:
    print("FAIL:", line)
print(f"checked {len(tables)} linked symbol tables"
      f"{'' if bad else ' — all consistent with their object decks'}")
sys.exit(1 if bad else 0)
