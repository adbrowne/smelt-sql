#!/usr/bin/env python3
"""Harvest `smelt explain --json` data for the Maintenance Atlas.

Walks the shipped example workspaces plus the maintenance-testkit's staged
recipe specimens (see `stage_atlas.rs`), running `smelt explain --json` (whole
graph) and `smelt explain <model> --json --show-sql` (per model) over each,
and writes the combined result as a single JSON blob.

Per-cell locality/scan-clamps/write-pattern text isn't in the JSON output yet
— `enrich.py` parses it out of the plain-text report and merges it in.
"""
import argparse
import datetime
import json
import os
import subprocess
import sys

RECIPES = [
    "value_enriched",
    "mutable_enriched",
    "keyed_additive",
    "keyed_order_monotone",
    "keyed_snapshot_overwrite",
]
EXAMPLE_PROJECTS = ["timeseries", "web_analytics", "retail_analytics"]


def run(smelt_bin, args, cwd, env):
    r = subprocess.run(
        [smelt_bin] + args, cwd=cwd, env=env, capture_output=True, text=True, timeout=300
    )
    return r.returncode, r.stdout, r.stderr


def harvest_project(smelt_bin, cwd, env):
    code, out, err = run(smelt_bin, ["explain", "--json"], cwd, env)
    if code != 0:
        return None, f"graph explain failed: {err[-500:]}"
    graph = json.loads(out)
    entry = {"graph": graph, "models": {}}
    for name in sorted(graph.get("models", {}).keys()):
        code, out, err = run(smelt_bin, ["explain", name, "--json", "--show-sql"], cwd, env)
        if code != 0:
            entry["models"][name] = {
                "no_plan": True,
                "reason": (err or out).strip().split("\n")[-1][:300],
            }
            continue
        try:
            plan = json.loads(out)
        except json.JSONDecodeError:
            entry["models"][name] = {"no_plan": True, "reason": "non-json output"}
            continue
        tcode, tout, _terr = run(smelt_bin, ["explain", name], cwd, env)
        plan["text_report"] = tout if tcode == 0 else None
        entry["models"][name] = plan
    return entry, None


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--smelt-bin", required=True, help="path to the built smelt CLI binary")
    ap.add_argument("--repo-root", required=True, help="repo root, containing examples/")
    ap.add_argument(
        "--recipes-dir",
        required=True,
        help="dir staged by `cargo run -p smelt-maintenance-testkit --example stage_atlas`",
    )
    ap.add_argument("--branch", default="", help="branch name to stamp into the output")
    ap.add_argument("--out", required=True, help="output atlas_data.json path")
    args = ap.parse_args()

    env = dict(os.environ)
    lib_dir = os.environ.get("DUCKDB_LIB_DIR")
    if lib_dir:
        env["LD_LIBRARY_PATH"] = lib_dir + os.pathsep + env.get("LD_LIBRARY_PATH", "")

    proj_dirs = [(p, os.path.join(args.repo_root, "examples", p)) for p in EXAMPLE_PROJECTS] + [
        ("recipe:" + r, os.path.join(args.recipes_dir, r, "project")) for r in RECIPES
    ]

    atlas = {
        "generated_at": datetime.datetime.now(datetime.timezone.utc).isoformat(
            timespec="seconds"
        ),
        "branch": args.branch,
        "projects": {},
    }
    failed = []
    for proj, cwd in proj_dirs:
        if not os.path.isdir(cwd):
            print(f"[{proj}] missing project dir {cwd}, skipping", file=sys.stderr)
            failed.append(proj)
            continue
        entry, err = harvest_project(args.smelt_bin, cwd, env)
        if err:
            print(f"[{proj}] {err}", file=sys.stderr)
            failed.append(proj)
            continue
        atlas["projects"][proj] = entry
        n_plans = sum(1 for v in entry["models"].values() if not v.get("no_plan"))
        print(f"[{proj}] {len(entry['models'])} models, {n_plans} with maintenance plans")

    with open(args.out, "w") as f:
        json.dump(atlas, f)
    print("wrote", args.out, os.path.getsize(args.out), "bytes")

    if failed:
        print(f"failed projects: {failed}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
