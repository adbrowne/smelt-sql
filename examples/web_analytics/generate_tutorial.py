#!/usr/bin/env python3
"""Generate the web-analytics maintenance tutorial page.

Renders `docs-site/docs/examples/web-analytics-maintenance.md` from a fixed
template of prose sections plus fenced SQL/transcript blocks. Every fenced
block is produced by actually invoking the `smelt` CLI against
`examples/web_analytics` and capturing its real output — the embedded SQL is
never hand-written. Two sourcing modes are used, matching the two things a
reader wants to see:

  - `explain <model> --show-sql --json --period <p>`: the JSON `cells`
    array is walked to build a compact SQL block per cell (grouping
    statements that share a `transactional_group` under `BEGIN`/`COMMIT`),
    labelled with the cell's trigger so the upstream-model creation edges
    are visible model-by-model.
  - `backbuild <model> --start <s> --end <e> --dry-run`: the full stdout
    transcript is captured verbatim (chunk boundaries, compiled upstream
    reads, and the maintenance statements for each chunk).

Both remove a single cosmetic artifact: the compiler's SQL emitter reserves
a blank comment line (`-- ` padded to the original comment's width) ahead of
each restated source comment. Lines that are nothing but `--` once
whitespace-trimmed carry no information and are dropped from both sourcing
paths identically (`canonicalize`), so the embedded blocks stay readable
without editing any actual SQL content.

Usage:
    python3 generate_tutorial.py            # render and write the page
    python3 generate_tutorial.py --check    # exit non-zero if the committed
                                             # page would change
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parent.parent
PROJECT_DIR = SCRIPT_DIR
OUTPUT_PATH = REPO_ROOT / "docs-site" / "docs" / "examples" / "web-analytics-maintenance.md"

# --period / --start / --end are pinned so every printed literal is stable
# across regenerations regardless of when the generator runs. The dates
# fall inside the datagen event window (`datagen.yaml`: 60 days starting
# 2026-03-19) but the values are never used to look up real data —
# `--show-sql` and `--dry-run` never connect to a backend, so the choice of
# date has no bearing on whether the pipeline has actually been run.
EXPLAIN_PERIOD = "2026-04-10..2026-04-11"
BACKBUILD_START = "2026-04-01"
BACKBUILD_END = "2026-04-19"

# A single-day run pinned to the datagen day whose data actually realises the
# skew inversion described in prose: an event at 2026-05-04 00:03 extends a
# session rooted at 2026-05-03 23:47, one gap that straddles midnight. The
# run window is [D, D+1) = [2026-05-04, 2026-05-05); the derived output
# window this run computes is the skew inversion [D-1, D+2) =
# [2026-05-03, 2026-05-06), which is exactly the DELETE range the emitted
# block below shows.
CROSS_MIDNIGHT_PERIOD = "2026-05-04..2026-05-05"


def run_smelt(args: list[str]) -> str:
    """Run the smelt CLI (via `cargo run`, so it always matches the current
    source tree) against `examples/web_analytics` and return stdout."""
    env = os.environ.copy()
    env.setdefault("DUCKDB_LIB_DIR", os.path.expanduser("~/.local/lib/duckdb"))
    duckdb_lib = env["DUCKDB_LIB_DIR"]
    ld_path = env.get("LD_LIBRARY_PATH", "")
    if duckdb_lib not in ld_path.split(":"):
        env["LD_LIBRARY_PATH"] = duckdb_lib + (":" + ld_path if ld_path else "")

    cmd = [
        "cargo",
        "run",
        "-q",
        "-p",
        "smelt-cli",
        "--bin",
        "smelt",
        "--manifest-path",
        str(REPO_ROOT / "Cargo.toml"),
        "--",
        *args,
    ]
    result = subprocess.run(
        cmd,
        cwd=PROJECT_DIR,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"smelt {' '.join(args)} failed (exit {result.returncode}):\n{result.stderr}"
        )
    return result.stdout


def canonicalize(text: str) -> str:
    """Drop comment-placeholder lines that are only `--` once trimmed (a
    cosmetic emitter artifact — see module docstring)."""
    lines = [line for line in text.split("\n") if line.strip() != "--"]
    return "\n".join(lines).strip("\n")


def render_cells_sql(cells: list[dict]) -> str:
    """Build a compact SQL block from an `explain --show-sql --json` cells
    array: one section per cell (labelled by its trigger), statements
    sharing a `transactional_group` wrapped in `BEGIN`/`COMMIT`."""
    blocks = []
    for cell in cells:
        trigger = cell["trigger"]
        statements = cell.get("statements") or []
        if not statements:
            reason = cell.get("no_statements_reason")
            if reason:
                blocks.append(f"-- trigger: {trigger}\n-- (no statements: {reason})")
            continue

        lines = [f"-- trigger: {trigger}"]
        groups: list[tuple[int, list[str]]] = []
        for stmt in statements:
            gid = stmt["transactional_group"]
            if groups and groups[-1][0] == gid:
                groups[-1][1].append(stmt["sql"])
            else:
                groups.append((gid, [stmt["sql"]]))

        for _gid, sqls in groups:
            if len(sqls) > 1:
                lines.append("BEGIN")
                for sql in sqls:
                    for sql_line in sql.split("\n"):
                        lines.append("  " + sql_line if sql_line else "")
                lines.append("COMMIT")
            else:
                lines.append(sqls[0])
        blocks.append("\n".join(lines))
    return "\n\n".join(blocks)


def explain_block(model: str, period: str = EXPLAIN_PERIOD) -> str:
    args = ["explain", model, "--show-sql", "--json", "--period", period]
    stdout = run_smelt(args)
    data = json.loads(stdout)
    sql = canonicalize(render_cells_sql(data["cells"]))
    command = " ".join(args)
    return f"<!-- smelt-generate: {command} -->\n```sql\n{sql}\n```\n"


def backbuild_block(model: str) -> str:
    args = [
        "backbuild",
        model,
        "--start",
        BACKBUILD_START,
        "--end",
        BACKBUILD_END,
        "--dry-run",
    ]
    stdout = run_smelt([*args, "--database", str(PROJECT_DIR / "target" / "dev.duckdb")])
    transcript = canonicalize(stdout)
    command = " ".join(args)
    return f"<!-- smelt-generate: {command} -->\n```sql\n{transcript}\n```\n"


def render_page() -> str:
    events_parsed_sql = explain_block("silver.events_parsed")
    sessions_sql = explain_block("silver.sessions")
    cross_midnight_sessions_sql = explain_block(
        "silver.sessions", period=CROSS_MIDNIGHT_PERIOD
    )
    events_enriched_sql = explain_block("silver.events_enriched")
    backfill_transcript = backbuild_block("silver.events_parsed")

    return f"""# Web analytics: lateness, redelivery, and attribution (incremental)

A worked example of the maintenance-plan machinery on a realistic
lateness-and-attribution pipeline: an at-least-once event feed with
redelivered duplicates and a multi-day arrival lag, sessionization with a
first-touch campaign attribution window, and an event-grain enrichment that
joins two maintained upstream models back together. Every SQL block below
is the real output of `smelt explain --show-sql` / `smelt backbuild
--dry-run` against
[`examples/web_analytics/`](https://github.com/adbrowne/smelt-sql/tree/main/examples/web_analytics)
— generated by
[`generate_tutorial.py`](https://github.com/adbrowne/smelt-sql/tree/main/examples/web_analytics/generate_tutorial.py)
so it cannot drift from the emitters that actually run it.

This page walks the lateness/redelivery/attribution slice of the pipeline;
see [the identity-stitching example](web_analytics.md) for the
bronze→silver→gold `amplitude_id` lineage the two examples share.

## Redelivery and lateness: `silver.events_parsed`

The bronze feed is at-least-once: a small fraction of events arrive twice,
byte-identical except for `arrival_time`, and an event's ingestion clock can
trail its occurrence clock by up to three days. `silver.events_parsed`
resolves both in one pass — a `QUALIFY ROW_NUMBER() OVER (PARTITION BY
event_id ORDER BY arrival_time) = 1` keeps the earliest-arriving copy of
each duplicated event, and a Form B filter (`event_date BETWEEN
CAST(arrival_time AS DATE) - INTERVAL '3 days' AND CAST(arrival_time AS
DATE)`) declares the accepted three-day late-arrival window. The planner
reads that filter as a genuine three-day lookback on `bronze.raw_events`,
so a run touching day D also re-touches the `[D-3, D)` partitions —
re-absorbing a late arrival that had not yet landed when those partitions
were first written.

`smelt explain silver.events_parsed --show-sql` renders the maintenance
statements this model executes for a given window — a `DELETE`+`INSERT`
pair over the real literal window bounds, bracketed transactional since the
pair must commit together:

{events_parsed_sql}

## Session attribution and the max-session-length cap: `silver.sessions`

`silver.sessions` reconstructs one row per session under a 30-minute
inactivity rule, then attributes each session a `utm_campaign` from the
earliest non-NULL value among its own events within the first five minutes
of the session start — attribution beyond that window never overrides the
session's campaign, even on a session that runs longer. A session cannot
span more than one day (the max-session-length cap): a `HAVING` clause
restates that cap as an explicit, checkable assertion in the emitted SQL,
and an accompanying Form B filter on `session_start_date` widens the read
of `silver.events_parsed` by the same one-day cap so a session that starts
late one day and crosses midnight is reconstructed as a single row rather
than split at the partition boundary.

{sessions_sql}

### The derived output window: a cross-midnight rewrite of the prior-day partition

`session_start_date` is not the column that decides which day's data can
still change a session — `silver.events_parsed.event_date` is, and the two
skew apart whenever a session starts late in one day and keeps accumulating
events into the next. The `WHERE event_date BETWEEN session_start_date -
INTERVAL '1 day' AND session_start_date + INTERVAL '1 day'` filter in the
model above is a **Form B relation**: it declares that bound explicitly, in
the model's own SQL, rather than leaving it to be assumed. The planner reads
that relation and **derives** the output window from it instead of using the
run window verbatim — for a `[D, D+1)` run this is the relation's
**skew inversion**, `[D−1, D+2)`: the run's own partition, plus the one
immediately before it (a session rooted the day before can still be
extended) and the one immediately after (recomputed for symmetry, though a
session can never start after its own events, so that side always comes back
unchanged).

Concretely: an event at `2026-05-04 00:03` is a 16-minute gap from the same
device's previous event at `2026-05-03 23:47` — well inside the session's
30-minute inactivity rule, so it extends that session rather than starting a
new one. A run over `[2026-05-04, 2026-05-05)` — the day the new event
arrives, not the day the session started — must still rewrite the
`2026-05-03` partition to fold the new event into the existing session row.
Running the derived output window instead of the run window verbatim is
exactly what makes that rewrite happen: the `DELETE`/`INSERT` pair below,
generated for that single-day run, covers `session_start_date` in
`[2026-05-03, 2026-05-06)` — the prior day included — not just
`[2026-05-04, 2026-05-05)`.

{cross_midnight_sessions_sql}

The derived output window is a range to be **covered**, not a mandate for a
single wide statement: write-size control stays available through backfill
chunking, which splits a run spanning many days into sequential
`DELETE`/`INSERT` pairs the same way it splits an ordinary wide run window,
each chunk's scan sized from that chunk's own reach rather than from the
whole range at once. Skew and chunk width compose independently — a
multi-day backfill over this model still emits one bounded chunk at a time,
each carrying its own one-day skew inversion at its edges.

## Event-grain enrichment and upstream-model edges: `silver.events_enriched`

`silver.events_enriched` joins each event's `session_id` and the session's
attributed `utm_campaign` back onto the event row, next to the event's own
raw `utm_campaign` for comparison. It has two maintained upstreams —
`silver.events_parsed` (this model's own `event_date` clock, read 1:1) and
`silver.sessions` (clocked by `session_start_date`, joined across the
session boundary) — and `explain` derives a creation-trigger cell for each
one, clamped by that upstream's own derived reach: a run touching one
`event_date` partition of `silver.events_parsed` re-touches only the
corresponding `event_date` partition of `silver.events_enriched`, never the
whole table, and the same holds for a `silver.sessions` update propagating
forward.

The three cells below are the `Backfill` catch-all plus one creation cell
per upstream — each printing its own `DELETE`+`INSERT` pair:

{events_enriched_sql}

## Backfilling a range: `smelt backbuild --dry-run`

`smelt backbuild <model> --start <date> --end <date> --dry-run` shows what
a backfill over a wider range would execute without touching a backend. A
range wide enough to exceed one batch is split into chunks, each introduced
by a `-- chunk k/N: [start, end)` boundary comment, printed in real
execution order — compiled upstream reads first, then each chunk's
maintenance statements for the target model:

{backfill_transcript}

## Where to look

- The models: [`silver/events_parsed.sql`](models/silver/events_parsed.sql),
  [`silver/sessions.sql`](models/silver/sessions.sql),
  [`silver/events_enriched.sql`](models/silver/events_enriched.sql).
- The datagen fixture producing the redelivered/late-arriving feed:
  [`datagen.yaml`](datagen.yaml).
- `docs/specs/maintenance_plan.md` — the plan/statement machinery this page
  demonstrates. `docs/specs/cli.md` — `explain --show-sql` and `backbuild
  --dry-run` surface reference.
"""


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--check",
        action="store_true",
        help="exit non-zero if the committed page differs from a fresh render",
    )
    args = parser.parse_args()

    rendered = render_page()

    if args.check:
        existing = OUTPUT_PATH.read_text() if OUTPUT_PATH.exists() else None
        if existing != rendered:
            print(
                f"{OUTPUT_PATH} is stale relative to a fresh render; "
                "run `python3 generate_tutorial.py` to regenerate it.",
                file=sys.stderr,
            )
            return 1
        print(f"{OUTPUT_PATH} is fresh.")
        return 0

    OUTPUT_PATH.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT_PATH.write_text(rendered)
    print(f"wrote {OUTPUT_PATH}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
