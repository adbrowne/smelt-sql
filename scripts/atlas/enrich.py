#!/usr/bin/env python3
"""Parse text_report per-cell extras (locality, clamps, write patterns) into the JSON cells.

`smelt explain --json` doesn't (yet) carry locality/scan-clamp/write-pattern
detail per cell — only the plain-text report does. This merges that text into
each cell as `extras`, then drops the now-redundant text_report blob.
"""
import argparse
import json
import re
import sys

FIELDS = {
    "ledger_catch_up": "ledger_catch_up",
    "locality": "locality",
    "scan clamps": "scan_clamps",
    "admissible write patterns": "write_patterns",
    "write pin": "write_pin",
}


def parse_cells(text):
    cells = []
    cur = None
    for line in text.splitlines():
        m = re.match(r"\s+- group (.+) on trigger (.+)$", line)
        if m:
            cur = {}
            cells.append(cur)
            continue
        if cur is None:
            continue
        m = re.match(r"\s+([a-z_ ]+):\s+(.*)$", line)
        if m and m.group(1).strip() in FIELDS:
            cur[FIELDS[m.group(1).strip()]] = m.group(2).strip()
    return cells


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--in", dest="inp", required=True, help="atlas_data.json from harvest.py")
    ap.add_argument("--out", required=True, help="enriched output path")
    args = ap.parse_args()

    d = json.load(open(args.inp))

    n = 0
    for proj, e in d["projects"].items():
        for name, mm in e["models"].items():
            if mm.get("no_plan") or not mm.get("text_report"):
                continue
            extras = parse_cells(mm["text_report"])
            if len(extras) != len(mm["cells"]):
                print(
                    f"WARN {proj}/{name}: {len(extras)} text cells vs {len(mm['cells'])} json cells",
                    file=sys.stderr,
                )
                continue
            for cell, ex in zip(mm["cells"], extras):
                cell["extras"] = ex
                n += 1
            del mm["text_report"]

    for proj, e in d["projects"].items():
        deps = {k: v.get("dependencies", []) for k, v in e["graph"]["models"].items()}
        ne = sum(len(v) for v in deps.values())
        print(f"{proj}: {len(deps)} models, {ne} dep edges")

    json.dump(d, open(args.out, "w"))
    print(f"enriched {n} cells -> {args.out}")


if __name__ == "__main__":
    main()
