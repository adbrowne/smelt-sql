#!/usr/bin/env python3
# NOTE: This script is superseded as the authoritative CI gate by the Rust
# integration test at crates/smelt-cli/tests/per_partition_equivalence.rs,
# which runs automatically under `cargo test -p smelt-cli`.  This script is
# retained for human convenience (configurable --days / --scale-factor flags)
# but is no longer required for CI.
"""Verify the incremental and full-rebuild pipelines agree on the local-only
columns, and document the expected as-of-day-D divergence on the global
identity columns.

The two identity algorithms `backward_fill` and `connected_components` are
*global* — their per-device output depends on the cumulative (device, user)
edge set across all dates.  When the day-by-day incremental pipeline writes
`gold/eventstream_with_identity` for day D, it freezes the global identity
mapping using only the edges visible up to D.  A subsequent day D+1 may
introduce edges that would have changed the day-D mapping, but the day-D
rows are not retroactively rewritten under DELETE+INSERT-per-partition.

A full-window single-shot rebuild, by contrast, writes every event with the
final cumulative identity in one pass.  So the two pipelines necessarily
diverge on the backward_fill / connected_components columns.  This is not a
bug — it mirrors how production streaming pipelines emit "as-of" daily
metrics rather than backfilling history every run.

Columns that DO match (this script asserts so):
  - total_events, event_date
  - dau_raw, identified_events_raw                     (no identity at all)
  - dau_forward_only, identified_events_forward_only   (per-session, local)

Columns that are EXPECTED to differ:
  - dau_backward_fill, identified_events_backward_fill         (global)
  - dau_connected_components, identified_events_connected_components (global)

Also asserts the `silver.events_parsed` dedup/lateness invariant: zero
duplicate `event_id`s in either pipeline's result, and an equal count of
accepted-lateness events (`arrival_time` within 3 days of `event_time`)
present in their own `event_date` partition across both pipelines.

Also asserts `silver.sessions`'s campaign-attribution and max-session-length
invariants: every session's `utm_campaign` equals the earliest non-NULL
campaign among its own events within the first 5 minutes of session start
(zero mismatches against an independently-computed golden query), zero
sessions exceed the explicit max-session-length cap, and the session count
agrees across both pipelines.

Also asserts `silver.events_enriched`'s event-grain enrichment: the set of
`(event_id, session_id, session_utm_campaign)` rows per `event_date`
partition matches exactly between the full-window rebuild and the
day-by-day replay.

Requires `smelt-datagen`, `smelt`, `duckdb` on PATH; Python 3.9+.
"""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
from datetime import date, timedelta
from pathlib import Path

EXAMPLE_DIR = Path(__file__).resolve().parent

DEFAULT_START = date(2026, 3, 19)
DEFAULT_DAYS = 60
DEFAULT_SCALE = 0.01

LOCAL_DAU_COLS = (
    "event_date",
    "total_events",
    "dau_raw",
    "dau_forward_only",
    "identified_events_raw",
    "identified_events_forward_only",
)
GLOBAL_DAU_COLS = (
    "dau_backward_fill",
    "dau_connected_components",
    "identified_events_backward_fill",
    "identified_events_connected_components",
)


def run_or_die(cmd: list[str], *, cwd: Path | None = None) -> None:
    result = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True)
    if result.returncode != 0:
        sys.stderr.write(f"\n[FAIL] command exited {result.returncode}\n")
        sys.stderr.write(f"  cmd: {' '.join(cmd)}\n")
        if result.stdout:
            sys.stderr.write(f"  stdout:\n{result.stdout}\n")
        if result.stderr:
            sys.stderr.write(f"  stderr:\n{result.stderr}\n")
        sys.exit(result.returncode)


def reset_target() -> None:
    target_db = EXAMPLE_DIR / "target" / "dev.duckdb"
    if target_db.exists():
        target_db.unlink()
    (EXAMPLE_DIR / "target").mkdir(exist_ok=True)


def datagen(scale_factor: float) -> None:
    data_dir = EXAMPLE_DIR / "data"
    if data_dir.exists():
        shutil.rmtree(data_dir)
    run_or_die(
        [
            "smelt-datagen",
            "--config",
            "datagen.yaml",
            "--scale-factor",
            str(scale_factor),
        ],
        cwd=EXAMPLE_DIR,
    )


def setup_sources() -> None:
    setup_sql = (EXAMPLE_DIR / "setup_sources.sql").read_text()
    proc = subprocess.run(
        ["duckdb", str(EXAMPLE_DIR / "target" / "dev.duckdb")],
        input=setup_sql,
        cwd=EXAMPLE_DIR,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        sys.stderr.write(f"\n[FAIL] setup_sources.sql failed:\n{proc.stderr}\n")
        sys.exit(proc.returncode)


def query_json(sql: str) -> list[dict]:
    proc = subprocess.run(
        ["duckdb", "-json", str(EXAMPLE_DIR / "target" / "dev.duckdb"), "-c", sql],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        sys.stderr.write(f"\n[FAIL] duckdb query: {sql}\n{proc.stderr}\n")
        sys.exit(proc.returncode)
    return json.loads(proc.stdout) if proc.stdout.strip() else []


def pipeline_a_full_window(start: date, end_exclusive: date) -> None:
    run_or_die(
        [
            "smelt",
            "run",
            "--event-time-start",
            start.isoformat(),
            "--event-time-end",
            end_exclusive.isoformat(),
        ],
        cwd=EXAMPLE_DIR,
    )


def pipeline_b_day_by_day(start: date, end_exclusive: date) -> None:
    """Replay the datagen window one non-overlapping single-day `[D, D+1)`
    window per iteration — mirrors `run_incremental.py` and the Rust
    `per_partition_equivalence` harness's `DAY_WINDOWS` scheme.

    An earlier version of this driver used an overlapping 2-day-lookback
    window (`[D-1, D+1)`) per iteration. That schedule predates
    `silver.device_user_edges`, an additive-fold keyed model
    (`grain: key`); its transactional merge ledger refuses to re-fold a
    partition it has already merged (`docs/specs/keyed_models.md`
    §"Reprocessing" / §"The transactional merge ledger" —
    `KeyedReprocessedWindow`), and the overlapping schedule would double-fold
    day D on both day D's and day D+1's window. Non-overlapping windows are
    the correct replay schedule; models that need a wider source read (e.g.
    `gold/identity_forward_only`, `silver/sessions`) declare their lookback
    via a Form B date filter, and the planner widens the read accordingly.
    """
    d = start
    while d < end_exclusive:
        window_start = d
        window_end = d + timedelta(days=1)
        run_or_die(
            [
                "smelt",
                "run",
                "--event-time-start",
                window_start.isoformat(),
                "--event-time-end",
                window_end.isoformat(),
            ],
            cwd=EXAMPLE_DIR,
        )
        d += timedelta(days=1)


def assert_local_columns_match(rows_a: list[dict], rows_b: list[dict]) -> None:
    """Local columns must be byte-equal across the two pipelines."""
    failures: list[str] = []
    if len(rows_a) != len(rows_b):
        failures.append(
            f"row count differs: pipeline A={len(rows_a)}, pipeline B={len(rows_b)}"
        )
    for i, (ra, rb) in enumerate(zip(rows_a, rows_b)):
        for col in LOCAL_DAU_COLS:
            if ra.get(col) != rb.get(col):
                failures.append(
                    f"row {i} ({ra.get('event_date')}): column '{col}' differs — "
                    f"A={ra.get(col)} vs B={rb.get(col)}"
                )
    if failures:
        print()
        print("=== LOCAL-COLUMN MISMATCH (UNEXPECTED) ===")
        for f in failures:
            print(f"  {f}")
        sys.exit(1)


def report_global_column_divergence(rows_a: list[dict], rows_b: list[dict]) -> None:
    """Global columns are expected to differ.  Print a summary of how much."""
    diffs_by_col: dict[str, int] = {col: 0 for col in GLOBAL_DAU_COLS}
    for ra, rb in zip(rows_a, rows_b):
        for col in GLOBAL_DAU_COLS:
            if ra.get(col) != rb.get(col):
                diffs_by_col[col] += 1
    print()
    print("=== global identity column divergence (expected) ===")
    for col, count in diffs_by_col.items():
        print(f"  {col}: {count}/{len(rows_a)} rows differ between pipelines")


def query_events_parsed_stats() -> dict:
    """Dedup/lateness stats for `main.silver_events_parsed` in the *current*
    `target/dev.duckdb`. Call this right after a pipeline finishes, before
    the next `reset_target()` call wipes the database."""
    duplicate_event_ids = query_json(
        "SELECT COUNT(*) AS n FROM ("
        "  SELECT event_id FROM main.silver_events_parsed"
        "  GROUP BY event_id HAVING COUNT(*) > 1"
        ")"
    )[0]["n"]
    accepted_late_present = query_json(
        "SELECT COUNT(*) AS n FROM ("
        "  SELECT DISTINCT event_id, event_date FROM raw.events"
        "  WHERE CAST(arrival_time AS TIMESTAMP)"
        "      <= CAST(event_time AS TIMESTAMP) + INTERVAL '3 days'"
        ") accepted"
        " WHERE EXISTS ("
        "   SELECT 1 FROM main.silver_events_parsed p"
        "   WHERE p.event_id = accepted.event_id"
        "     AND p.event_date = CAST(accepted.event_date AS DATE)"
        " )"
    )[0]["n"]
    return {
        "duplicate_event_ids": duplicate_event_ids,
        "accepted_late_events_present": accepted_late_present,
    }


def query_session_stats() -> dict:
    """Campaign-attribution and max-session-length stats for
    `main.silver_sessions` in the *current* `target/dev.duckdb`. Call this
    right after a pipeline finishes, before the next `reset_target()` call
    wipes the database."""
    attribution_mismatches = query_json(
        "SELECT COUNT(*) AS n FROM ("
        "  SELECT s.session_id, s.utm_campaign AS actual, ("
        "    SELECT e.utm_campaign FROM main.silver_events_parsed e"
        "    WHERE e.device_id = s.device_id"
        "      AND e.event_ts >= CAST(s.session_start AS TIMESTAMP)"
        "      AND e.event_ts <= CAST(s.session_end AS TIMESTAMP)"
        "      AND e.event_ts <= CAST(s.session_start AS TIMESTAMP) + INTERVAL '5 minutes'"
        "      AND e.utm_campaign IS NOT NULL"
        "    ORDER BY e.event_ts ASC LIMIT 1"
        "  ) AS expected"
        "  FROM main.silver_sessions s"
        ") mismatch WHERE actual IS DISTINCT FROM expected"
    )[0]["n"]
    cap_violations = query_json(
        "SELECT COUNT(*) AS n FROM main.silver_sessions"
        " WHERE session_end - session_start > INTERVAL '1 day'"
    )[0]["n"]
    session_count = query_json("SELECT COUNT(*) AS n FROM main.silver_sessions")[0]["n"]
    return {
        "attribution_mismatches": attribution_mismatches,
        "cap_violations": cap_violations,
        "session_count": session_count,
    }


def assert_session_attribution_and_cap(stats_a: dict, stats_b: dict) -> None:
    """`silver.sessions`'s campaign-attribution (first-5-minutes, earliest
    non-NULL `utm_campaign`) and explicit max-session-length cap invariants,
    checked on both pipelines, plus a lightweight cross-pipeline session-count
    equivalence signal (the Rust harness asserts the full
    `(session_id, utm_campaign)` set equality)."""
    failures: list[str] = []
    for label, stats in (("A (full rebuild)", stats_a), ("B (day-by-day)", stats_b)):
        if stats["attribution_mismatches"] != 0:
            failures.append(
                f"pipeline {label}: {stats['attribution_mismatches']} session(s) "
                f"with utm_campaign attribution not matching the earliest "
                f"non-NULL campaign among events within the first 5 minutes"
            )
        if stats["cap_violations"] != 0:
            failures.append(
                f"pipeline {label}: {stats['cap_violations']} session(s) exceed "
                f"the explicit max-session-length cap (1 day)"
            )
    if stats_a["session_count"] != stats_b["session_count"]:
        failures.append(
            "session count differs between pipelines: "
            f"A={stats_a['session_count']} B={stats_b['session_count']}"
        )
    if failures:
        print()
        print("=== SESSION ATTRIBUTION/CAP MISMATCH (UNEXPECTED) ===")
        for f in failures:
            print(f"  {f}")
        sys.exit(1)


def query_events_enriched_by_partition() -> dict[str, set[tuple]]:
    """`(event_id, session_id, session_utm_campaign)` rows grouped by
    `event_date` partition, from the *current* `target/dev.duckdb`. Call
    right after a pipeline finishes, before the next `reset_target()` wipes
    the database."""
    rows = query_json(
        "SELECT event_date::VARCHAR AS event_date, event_id, session_id, "
        "session_utm_campaign FROM main.silver_events_enriched"
    )
    by_partition: dict[str, set[tuple]] = {}
    for row in rows:
        by_partition.setdefault(row["event_date"], set()).add(
            (row["event_id"], row["session_id"], row["session_utm_campaign"])
        )
    return by_partition


def assert_events_enriched_matches(
    by_partition_a: dict[str, set[tuple]], by_partition_b: dict[str, set[tuple]]
) -> None:
    """`silver.events_enriched` must match exactly, partition by partition,
    between the full-window rebuild and the day-by-day replay — its
    creation cells over both model upstreams (`silver.events_parsed`,
    `silver.sessions`) are `RecomputeRegion`/`DeleteInsert`, so per-partition
    equivalence is the hard invariant."""
    failures: list[str] = []
    if by_partition_a.keys() != by_partition_b.keys():
        failures.append(
            f"partition sets differ: A={sorted(by_partition_a)} "
            f"B={sorted(by_partition_b)}"
        )
    for partition, rows_a in by_partition_a.items():
        rows_b = by_partition_b.get(partition, set())
        if rows_a != rows_b:
            failures.append(
                f"partition {partition}: {len(rows_a)} rows in A, "
                f"{len(rows_b)} rows in B (row sets differ)"
            )
    if failures:
        print()
        print("=== EVENTS_ENRICHED MISMATCH (UNEXPECTED) ===")
        for f in failures:
            print(f"  {f}")
        sys.exit(1)


def assert_dedup_and_lateness(stats_a: dict, stats_b: dict) -> None:
    """`silver.events_parsed`'s redelivery-dedup and 3-day late-window
    acceptance invariants, checked on both pipelines."""
    failures: list[str] = []
    for label, stats in (("A (full rebuild)", stats_a), ("B (day-by-day)", stats_b)):
        if stats["duplicate_event_ids"] != 0:
            failures.append(
                f"pipeline {label}: {stats['duplicate_event_ids']} duplicate "
                f"event_id(s) in silver.events_parsed (expected 0 — redelivery "
                f"dedup should have collapsed them)"
            )
    if stats_a["accepted_late_events_present"] != stats_b["accepted_late_events_present"]:
        failures.append(
            "accepted-lateness event presence count differs between pipelines: "
            f"A={stats_a['accepted_late_events_present']} "
            f"B={stats_b['accepted_late_events_present']}"
        )
    if failures:
        print()
        print("=== DEDUP/LATENESS MISMATCH (UNEXPECTED) ===")
        for f in failures:
            print(f"  {f}")
        sys.exit(1)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--start-date",
        type=lambda s: date.fromisoformat(s),
        default=DEFAULT_START,
    )
    parser.add_argument("--days", type=int, default=DEFAULT_DAYS)
    parser.add_argument("--scale-factor", type=float, default=DEFAULT_SCALE)
    args = parser.parse_args()

    end_exclusive = args.start_date + timedelta(days=args.days)

    print(f"[datagen] scale_factor={args.scale_factor}")
    reset_target()
    datagen(args.scale_factor)
    setup_sources()

    print(
        f"[A] full-window rebuild [{args.start_date.isoformat()} .. {end_exclusive.isoformat()})"
    )
    pipeline_a_full_window(args.start_date, end_exclusive)
    rows_a = query_json(
        "SELECT * FROM main.marts_daily_active_users_by_method ORDER BY event_date"
    )
    stats_a = query_events_parsed_stats()
    session_stats_a = query_session_stats()
    events_enriched_a = query_events_enriched_by_partition()

    reset_target()
    setup_sources()
    print(f"[B] day-by-day replay ({args.days} days, single-day window each)")
    pipeline_b_day_by_day(args.start_date, end_exclusive)
    rows_b = query_json(
        "SELECT * FROM main.marts_daily_active_users_by_method ORDER BY event_date"
    )
    stats_b = query_events_parsed_stats()
    session_stats_b = query_session_stats()
    events_enriched_b = query_events_enriched_by_partition()

    # The local columns must match exactly.  This is the hard invariant the
    # day-by-day pipeline preserves.
    assert_local_columns_match(rows_a, rows_b)

    # silver.events_parsed's redelivery-dedup and 3-day late-window
    # acceptance invariants must hold identically in both pipelines.
    assert_dedup_and_lateness(stats_a, stats_b)

    # silver.sessions's campaign-attribution and max-session-length
    # invariants must hold identically in both pipelines.
    assert_session_attribution_and_cap(session_stats_a, session_stats_b)

    # silver.events_enriched's event-grain enrichment must match exactly,
    # partition by partition.
    assert_events_enriched_matches(events_enriched_a, events_enriched_b)

    # Global columns are expected to differ — that's the "as-of-day-D" property
    # of incremental pipelines with global identity.  Print a summary so the
    # user can see how much.
    report_global_column_divergence(rows_a, rows_b)

    print()
    print(
        f"=== local-column equivalence: PASS ({len(rows_a)} rows; "
        f"{len(LOCAL_DAU_COLS)} cols agree) ==="
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
