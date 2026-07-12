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

# The never-idle comparison table (design doc §"The lesson") quotes the
# exact session counts the real e2e fixtures pin, never hand-arithmetic:
#   - `cross_midnight_rebase.rs::never_idle_device_yields_one_session_per_day`
#     — the clock-anchored `silver.sessions` table settles into 9 sessions
#     over the same 447-event, 10-calendar-day never-idle fixture.
#   - `cross_midnight_rebase.rs::chained_never_idle_device_yields_one_session_per_two_days`
#     — the root-anchored `silver.sessions_chained` table settles into 5
#     sessions over the identical 447-event fixture.
NEVER_IDLE_EVENT_COUNT = 447
NEVER_IDLE_CLOCK_SESSION_COUNT = 9
NEVER_IDLE_CHAINED_SESSION_COUNT = 5


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
    sessions_chained_sql = explain_block("silver.sessions_chained")
    events_enriched_sql = explain_block("silver.events_enriched")
    backfill_transcript = backbuild_block("silver.events_parsed")

    return f"""# Web analytics: lateness, redelivery, and attribution (incremental)

A worked example of the maintenance-plan machinery on a realistic
lateness-and-attribution pipeline: an at-least-once event feed with
redelivered duplicates and a multi-day arrival lag, two sessionizers built
around the same 30-minute inactivity rule but differing in *where their
session-length cap's phase comes from*, and an event-grain enrichment that
joins all of it back onto the event row. Every SQL block below is the real
output of `smelt explain --show-sql` / `smelt backbuild --dry-run` against
[`examples/web_analytics/`](https://github.com/adbrowne/smelt-sql/tree/main/examples/web_analytics)
— generated by
[`generate_tutorial.py`](https://github.com/adbrowne/smelt-sql/tree/main/examples/web_analytics/generate_tutorial.py)
so it cannot drift from the emitters that actually run it.

This page walks the lateness/redelivery/attribution/sessionization slice of
the pipeline; see [the identity-stitching example](web_analytics.md) for the
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

## Any bounded sessionizer must cut somewhere

The obvious sessionizer never stops growing a session: chain events by a
30-minute inactivity rule with no other limit, and one device that never
goes idle for 30 minutes — a background sync, a kiosk display, a
misbehaving client — produces a single session spanning the device's entire
history. That is unbounded in both directions that matter for an
incremental pipeline: the session's row can never be considered final (a new
event next year still extends it), and reconstructing it costs a full-history
scan rather than a bounded lookback. **Any sessionizer that stays
incrementally maintainable must cut a long-running chain somewhere** — the
question is not whether to cut, but where the cut's *phase* comes from.

Two designs below answer that question differently, and the answer decides
the model's execution shape:

- **The clock decides.** A session dies at a calendar boundary — computable
  from the event's own timestamp, no memory of the session's own history
  required. This is `silver.sessions`: every partition is a pure function of
  a bounded trailing window of *source* events, so partitions build in any
  order, in parallel — window-independent.
- **The session's own root decides.** A session dies when it has run too
  long *since it started*, and "too long" is measured from a root timestamp
  that was fixed whenever this session first began — months ago, for a
  long-lived session. No bounded read of new events alone can recover that
  root; the model must ask itself "which session am I continuing, and when
  did it start?" This is `silver.sessions_chained`: it reads its own prior
  output, and the planner proves that self-read converges only if partitions
  build strictly in temporal order — ordered, sequential, never parallel.

Both tables below keep the identical 30-minute inactivity + platform-boundary
rule and the identical 5-minute first-touch campaign attribution; they
differ in nothing but where the cap's phase comes from — which is exactly
what makes the comparison a clean before/after on execution shape rather than
a comparison muddied by unrelated rule differences.

## The clock-anchored cut: `silver.sessions`

`silver.sessions` cuts a session at the midnight ending day D **iff the
session has an event in the first 30 minutes of day D** (`[D 00:00, D
00:30)`). That single rule has two consequences worth deriving explicitly,
because they are what make the table window-independent: a session rooted
before `00:30` dies at its own day's end (nothing keeps it open past
midnight, since it needed an event before `00:30` the next day to survive,
and its own root is already past that time); a session rooted at or after
`00:30` may cross **one** midnight, but a crossing session always has an
event before `00:30` of the new day (the crossing gap itself is at most the
30-minute inactivity threshold), so it always dies at the **second**
midnight. Every session therefore spans at most two calendar days — under
48 hours — and the closed form is checkable purely from the root's own
time-of-day: deadline = `end_of_day(date(root))` if `time(root) < 00:30`,
else `end_of_day(date(root) + 1 day)`.

`silver.sessions` reconstructs one row per session under the 30-minute
inactivity rule plus that clock-anchored deadline, then attributes each
session a `utm_campaign` from the earliest non-NULL value among its own
events within the first five minutes of the session start — attribution
beyond that window never overrides the session's campaign, even on a
session that runs longer. The `HAVING` clause restates the ≤2-day span as an
explicit, checkable assertion in the emitted SQL, and an accompanying Form B
filter on `session_start_date` widens the read of `silver.events_parsed` by
one day so a session that starts late one day and crosses midnight is
reconstructed as a single row rather than split at the partition boundary.

{sessions_sql}

### The derived output window: a cross-midnight rewrite of the prior-day partition

`session_start_date` is not the column that decides which day's data can
still change a session — `silver.events_parsed.event_date` is, and the two
skew apart whenever a session starts late in one day and keeps accumulating
events into the next. The `WHERE event_date BETWEEN session_start_date AND
session_start_date + INTERVAL '1 day'` filter in the model above is a **Form
B relation**: it declares that bound explicitly, in the model's own SQL,
rather than leaving it to be assumed. The planner reads that relation and
**derives** the output window from it instead of using the run window
verbatim — for a `[D, D+1)` run this is the relation's **skew inversion**,
`[D−1, D+2)`: the run's own partition, plus the one immediately before it (a
session rooted the day before can still be extended) and the one
immediately after (recomputed for symmetry, though a session can never start
after its own events, so that side always comes back unchanged).

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

### What about a session that spans two midnights?

The Form B filter above is not a heuristic that happens to catch most
cross-midnight sessions — it is a **semantic cap**, declared in the model's
own SQL, and it applies identically no matter how far a session's events
would otherwise chain. A session whose root's time-of-day is at or after
`00:30` reaches all the way to the *second* midnight before the cap can end
it, and it is truncated the same way whether the pipeline replays
day-by-day or rebuilds the whole range from scratch in one pass: the
relation, not the run shape, decides which rows contribute to which
partition.

Concretely (this dataset's browsing behaviour never happens to produce a
chain this long, so the numbers below come from a synthetic 60-event chain
injected into `raw.events` and run through the real pipeline —
`sessionize` and this model, unmodified — pinned end-to-end by
`crates/smelt-cli/tests/e2e/per_partition_equivalence.rs
::web_analytics_session_attribution_matches_full_rebuild`, rather than from
a `smelt-generate` block over the seeded data): one device emits a
continuous event chain from `2026-03-19 23:50` to `2026-03-21 00:15`, every
pairwise gap under 30 minutes, same platform throughout. No inactivity gap
ever breaks that chain, so the only thing that can end the session rooted
at `23:50` is the cap itself. The root's time-of-day (`23:50`) is `>= 00:30`,
so its deadline reaches to the *second* midnight (the start of
`2026-03-21`) — every event strictly before that deadline, the root plus
the full 25-minute day-2 grid, merges into one session: it carries
`event_count=59` and settles at `session_end=2026-03-20 23:55`, identically
in a day-by-day replay and in one full-range build. The remaining event is
not lost: `2026-03-21 00:15` falls at or past the deadline, strikes a new
root at its own timestamp, and surfaces as its own single-event session —
sessions never overlap, and every one of the 60 events is counted exactly
once (59 + 1).

That is exactly what makes the declaration sound as the planner's source of
truth for the derived output window: the relation holds for every row the
model can produce, at whatever granularity the run happens to replay it.

## The root-anchored cut: `silver.sessions_chained`

`silver.sessions_chained` answers the same "where must a bounded
sessionizer cut" question with a different rule: a day-D event continues an
open session only if that session **rooted less than two calendar days
ago**; otherwise it strikes a new root. The cutoff's phase is inherited from
the session's own root, decided whenever that session first began — for a
long-lived session that could be months back, and no bounded read of new
source events alone can recover it. The model must read **its own prior
output** to learn which sessions are still open and when they rooted, via a
backward-bounded (2-day, no forward reach) self-reference to
`smelt.silver.sessions_chained`.

That self-read is exactly what the planner's window-independence proof
keys off: a backward-bounded, non-forward-reaching self-reference proves
`Ordered` rather than `WindowIndependent` — the runtime is forced to build
this table's partitions strictly in temporal order, one partition per batch,
never in parallel, never as a single wide batch. Backfills of
`silver.sessions_chained` cannot be parallelised; that constraint is the
entire teaching point of the divergence from `silver.sessions`, not an
implementation accident. `explain` below shows a third cell absent from
every window-independent model in this pipeline — a `NewData` trigger on
`silver.sessions_chained` itself, the self-edge made visible as its own
maintenance-plan edge:

{sessions_chained_sql}

## Same rule, three fates: the never-idle comparison

A device that never idles for the full 30-minute inactivity threshold (say,
one event every 29 minutes, forever) never strikes a natural boundary after
its very first event — every cut it experiences comes entirely from
whichever table's cap fires. Feeding the identical {NEVER_IDLE_EVENT_COUNT}-event,
never-idle chain (rooted at `2024-01-01 14:00`, running 9 elapsed days)
through each design in turn, end to end through the real pipeline, produces
three different fates for what is, underneath, the same gap rule:

| Design | Sessions emitted | Execution |
|---|---|---|
| `silver.sessions_chained` (root-anchored cap) | {NEVER_IDLE_CHAINED_SESSION_COUNT} sessions (~1 session per 2 days) | ordered, sequential |
| `silver.sessions` (clock-anchored cap) | {NEVER_IDLE_CLOCK_SESSION_COUNT} sessions (~1 session per day) | window-independent, parallel |
| a frame-cap + `COALESCE` fallback design (never shipped) | ~50 single-event sessions per day (confetti) | "parallel", wrong |

The first two rows are pinned end to end against the real models —
`crates/smelt-cli/tests/e2e/cross_midnight_rebase.rs
::never_idle_device_yields_one_session_per_day` and
`::chained_never_idle_device_yields_one_session_per_two_days` — including
that day-by-day replay converges to the identical trajectory as a
from-scratch sequential build (in-order for the chained table, in any order
for the clock table), and that neither table ever degenerates to a
single-event session under this fixture.

The third row is narrated, not shipped — it is what a max-session-length cap
enforced purely inside `sessionize`'s own window frames, falling back to
`COALESCE(MAX(_boundary_ts) OVER (...), event_ts)` when the trailing frame
finds no boundary row, actually does under this same never-idle input: the
first day forms one capped session, and because the never-idle chain never
strikes another natural boundary, the trailing frame never again contains a
boundary row to fall back on — every subsequent event becomes its own
single-event session, one every 29 minutes, roughly 50 a day. Session count
is a headline web-analytics metric; a design that inflates it ~50× in the
presence of one weird device is the anti-pattern this pair of tables
replaces, not a variant worth keeping around as a third option.

Two near-identical business rules — clock-anchored and root-anchored — differ
in exactly one decision (where the cap's phase comes from), and that one
decision is what flips the execution property from parallel to ordered. The
frame-cap design differs in a much smaller, much more dangerous way: it looks
window-independent (no self-reference, no ordering requirement at all) right
up until a never-idle input reveals that its cap was never actually bounding
anything.

## Event-grain enrichment and upstream-model edges: `silver.events_enriched`

`silver.events_enriched` joins each event's session identity — under
**both** upstream tables' cut rules — back onto the event row, next to the
event's own raw `utm_campaign` for comparison. `session_id` /
`session_utm_campaign`, from the clock-anchored `silver.sessions`, are
primary — the gold identity models downstream consume these, keeping the
window-independent table on the hot path. `session_id_chained` /
`session_utm_campaign_chained`, from the root-anchored
`silver.sessions_chained`, are additive: the divergence between the two ids
on any given event is directly queryable, the comparison surface this page
has just walked.

Three maintained upstreams in one model body — `silver.events_parsed` (this
model's own `event_date` clock, read 1:1), `silver.sessions` (clocked by
`session_start_date`, joined across the session boundary, 1-day cap), and
`silver.sessions_chained` (same partition-column shape, 2-day cap) — and
`explain` derives a creation-trigger cell for each one, clamped by that
upstream's own derived reach: a run touching one `event_date` partition of
`silver.events_parsed` re-touches only the corresponding `event_date`
partition of `silver.events_enriched`, never the whole table, and the same
holds for a `silver.sessions` or `silver.sessions_chained` update
propagating forward.

The four cells below are the `Backfill` catch-all plus one creation cell per
upstream — each printing its own `DELETE`+`INSERT` pair:

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
  [`silver/sessions.sql`](models/silver/sessions.sql) +
  [`functions/sessionize.sql`](functions/sessionize.sql) (clock-anchored),
  [`silver/sessions_chained.sql`](models/silver/sessions_chained.sql)
  (root-anchored, self-referential),
  [`silver/events_enriched.sql`](models/silver/events_enriched.sql).
- The datagen fixture producing the redelivered/late-arriving feed:
  [`datagen.yaml`](datagen.yaml).
- The design doc behind the two-table split:
  [`docs/research/20260711-clock-vs-root-anchored-sessions.md`](https://github.com/adbrowne/smelt-sql/tree/main/docs/research/20260711-clock-vs-root-anchored-sessions.md).
- `docs/specs/maintenance_plan.md` — the plan/statement machinery this page
  demonstrates. `docs/specs/cli.md` — `explain --show-sql` and `backbuild
  --dry-run` surface reference. [The incremental-models guide's
  self-referential (ordered) models
  section](../guide/incremental-models.md#self-referential-ordered-models)
  — the general framework rule `silver.sessions_chained` is a real consumer
  of.
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
