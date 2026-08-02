#!/usr/bin/env python3
"""lnk_lite: overlay-link a single HALSFC object deck into a base FCM.

The real lnk101 is not publicly available; this tool closes the loop by
linking a standalone HAL/S program against the runtime library ALREADY
LAID OUT in a reference image (a HALSFC+lnk101-produced .fcm with its
symbols JSON, e.g. the Virtual AGC hello fixture). The program's CSECTs
overlay the base program's same-role CSECTs by name pattern ($0*, #E*,
#D*, START, with the generated @0* stack mapped onto the base stack);
externals (#Q* runtime stubs etc.) resolve against the base symbol
table. Trap addresses and entry point are unchanged, so the base
symbols JSON serves the linked output as-is.

Object-deck format per ASM101S/readObject101S.py & objectWriter.py:
80-byte cards, 0x02 + EBCDIC type; ESD symbols are 16-byte entries;
TXT: relativeAddress(bytes)/size/esdid/data; RLD entries are 8 bytes:
relId(2) posId(2) flags(1) address(3, bytes). Relocation semantics
recovered from the hello fixture's linked bytes:
  flags 0x00: halfword address constant = target's 16-bit address
              (bit 15 set + low 15 bits for addresses >= 0x8000, i.e.
              BSR-sector form; plain for sector-0 targets).
  flags 0x10: high half of a fullword pointer: sector-form address.
  flags 0x40: low half of that pointer: 0x0700 (C/CB/CD set) with
              BSV = target's sector in bits 8-11, DSV = 0.

Usage: lnk_lite.py PROG.obj BASE.fcm BASE.json -o OUT.fcm
"""

import json
import sys

E2A = {0xC1: 'A'}  # filled below


def ebcdic_to_ascii(bs):
    return bytes(bs).decode("cp037", errors="replace")


def be(bs):
    v = 0
    for b in bs:
        v = (v << 8) | b
    return v


def parse_deck(path):
    data = open(path, "rb").read()
    modules = []
    cur = {"esds": [], "txt": [], "rld": []}
    for off in range(0, len(data), 80):
        card = data[off:off + 80]
        if not card or card[0] != 0x02:
            continue
        typ = ebcdic_to_ascii(card[1:4])
        if typ == "ESD":
            size = be(card[10:12])
            for k in range(0, min(size, 48), 16):
                ent = card[16 + k:32 + k]
                name = ebcdic_to_ascii(ent[0:8]).rstrip()
                styp = ent[8]
                # type byte: 0x00 SD, 0x02 ER (per readObject101S flags)
                kind = "ER" if styp == 0x02 else "SD"
                cur["esds"].append({"name": name, "kind": kind})
        elif typ == "TXT":
            cur["txt"].append({
                "addr": be(card[5:8]),
                "size": be(card[10:12]),
                "esdid": be(card[14:16]),
                "data": card[16:16 + be(card[10:12])],
            })
        elif typ == "RLD":
            size = be(card[10:12])
            for j in range(size // 8):
                o = 16 + j * 8
                cur["rld"].append({
                    "rel": be(card[o:o + 2]),
                    "pos": be(card[o + 2:o + 4]),
                    "flags": card[o + 4],
                    "addr": be(card[o + 5:o + 8]),
                })
        elif typ == "END":
            modules.append(cur)
            cur = {"esds": [], "txt": [], "rld": []}
    return modules


def sector16(addr_hw):
    """16-bit encoding of a 19-bit halfword address (BSR-sector form
    above 32K, plain below)."""
    return (0x8000 | (addr_hw & 0x7FFF)) if addr_hw >= 0x8000 else addr_hw


def main():
    args = [a for a in sys.argv[1:] if a != "-o"]
    obj_path, base_fcm, base_json, out_fcm = args
    modules = parse_deck(obj_path)
    base = bytearray(open(base_fcm, "rb").read())
    symtab = json.load(open(base_json))
    sections = {s["name"]: s for s in symtab["sections"]}
    symbols = {s["name"]: s["address"] for s in symtab["symbols"]}

    def role_of(name):
        for pre in ("$0", "#E", "#D", "@0"):
            if name.startswith(pre):
                return pre
        return name  # e.g. START

    # Base addresses for each role, from the base image's own program
    base_roles = {}
    for name, sec in sections.items():
        r = role_of(name)
        if r in ("$0", "#E", "#D", "@0", "START") and r not in base_roles:
            base_roles.setdefault(r, (sec["address"], sec["size"]))
    # role_of maps START to itself
    if "START" in sections:
        base_roles["START"] = (sections["START"]["address"], sections["START"]["size"])

    for m, mod in enumerate(modules):
        # ESDID -> address (1-based, in deck order)
        addr_of = {}
        for i, esd in enumerate(mod["esds"], start=1):
            name = esd["name"]
            if esd["kind"] == "SD":
                r = role_of(name)
                if r not in base_roles:
                    sys.exit(f"no base region for CSECT {name} (role {r})")
                addr_of[i] = base_roles[r][0]
            else:
                r = role_of(name)
                if name in symbols:
                    addr_of[i] = symbols[name]
                elif name in sections:
                    addr_of[i] = sections[name]["address"]
                elif r in base_roles:
                    addr_of[i] = base_roles[r][0]
                else:
                    sys.exit(f"unresolved external {name}")
        # lay down text (addresses in bytes within CSECT)
        for t in mod["txt"]:
            byte_base = addr_of[t["esdid"]] * 2
            base[byte_base + t["addr"]:byte_base + t["addr"] + t["size"]] = t["data"]
        # relocate
        for r in mod["rld"]:
            target = addr_of[r["rel"]]
            spot = addr_of[r["pos"]] * 2 + r["addr"]
            if r["flags"] == 0x00 or r["flags"] == 0x10:
                val = sector16(target)
            elif r["flags"] == 0x40:
                val = 0x0700 | ((target >> 15) << 4)
            else:
                sys.exit(f"unknown RLD flags {r['flags']:02X}")
            old = be(base[spot:spot + 2])
            base[spot:spot + 2] = (old + val & 0xFFFF).to_bytes(2, "big")

    open(out_fcm, "wb").write(base)
    print(f"linked {obj_path} over {base_fcm} -> {out_fcm} "
          f"(entry {symtab['entryPoint']}, symbols unchanged)")


if __name__ == "__main__":
    main()
