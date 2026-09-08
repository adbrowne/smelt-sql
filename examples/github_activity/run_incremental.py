#!/usr/bin/env python3
"""Day-by-day incremental replay driver for examples/github_activity.

Unlike `examples/web_analytics/run_incremental.py`, there is no datagen step:
`seeds/github_events_sample.parquet` is a committed, reproducible export of
`sample.sql` (`refresh_sample.sh` regenerates it against BigQuery). What this
script does, in order:

  1. Wipe target/dev.duckdb and create the empty `raw.github_events` table
     via setup_sources.sql.
  2. Loop day-by-day across the fixture's 30-day range, invoking `smelt run
     --event-time-start D --event-time-end D+1`. The day's load itself is no
     longer this script's job: `models/sources/raw/github_loader.yml`
     declares `load_day.sh` as an external step producing both raw sources,
     and `smelt run` orders and invokes it ahead of every model that reads
     them (`docs/specs/sources.md` §"Externally-produced sources (black-box
     steps)"). Each day D's load appends the real rows whose `created_at`
     falls on day D, plus a deterministic 2% redelivery of day D-1's rows
     (`MOD(CAST(id AS BIGINT), 50) = 0`) — the loader's declared at-least-once
     behaviour. GitHub Archive itself carries no duplicate event ids, so
     without this the dedup model would never see a duplicate.
  3. Finish with `smelt test`.

Per-iteration output is one structured line; a final summary block reports
the total redelivered-row count observed (computed directly against the
committed parquet fixture with the loader's own predicate, since the load
itself now happens inside `smelt run`), so it is a number in run output
rather than a bare "passed".

Requires `smelt` and `duckdb` on PATH; Python 3.9+.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from dataclasses import dataclass
from datetime import date, timedelta
from pathlib import Path

EXAMPLE_DIR = Path(__file__).resolve().parent
SAMPLE_PARQUET = EXAMPLE_DIR / "seeds" / "github_events_sample.parquet"

# Matches sample.sql's pinned range (docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md).
DEFAULT_START = date(2026, 8, 5)
DEFAULT_DAYS = 30
REDELIVERY_MODULUS = 50  # MOD(CAST(id AS BIGINT), 50) = 0 -> ~2% of the previous day


@dataclass
class IterReport:
    iter_n: int
    total: int
    day: date
    redelivered_rows: int
    seconds: float


def daterange(start: date, days: int) -> list[date]:
    return [start + timedelta(days=i) for i in range(days)]


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


def query_scalar(db: Path, sql: str) -> int:
    proc = subprocess.run(
        ["duckdb", "-json", str(db), "-c", sql], capture_output=True, text=True
    )
    if proc.returncode != 0:
        sys.stderr.write(f"\n[FAIL] duckdb query: {sql}\n{proc.stderr}\n")
        sys.exit(proc.returncode)
    rows = json.loads(proc.stdout) if proc.stdout.strip() else []
    return int(next(iter(rows[0].values()))) if rows else 0


def setup_sources(db: Path) -> None:
    setup_sql = (EXAMPLE_DIR / "setup_sources.sql").read_text()
    proc = subprocess.run(
        ["duckdb", str(db)], input=setup_sql, cwd=EXAMPLE_DIR, capture_output=True, text=True
    )
    if proc.returncode != 0:
        sys.stderr.write(f"\n[FAIL] duckdb < setup_sources.sql exited {proc.returncode}\n")
        sys.stderr.write(f"  stderr:\n{proc.stderr}\n")
        sys.exit(proc.returncode)


def redelivered_count(db: Path, day: date) -> int:
    """Count of day `day - 1`'s rows load_day.sh redelivers into day `day`'s
    load — the loader's own predicate, computed directly against the
    committed parquet fixture (the load itself happens inside `smelt run`,
    driven by the declared external step, not by this script)."""
    prev = day - timedelta(days=1)
    return query_scalar(
        db,
        f"SELECT count(*) FROM read_parquet('{SAMPLE_PARQUET}') "
        f"WHERE CAST(created_at AS DATE) = DATE '{prev.isoformat()}' "
        f"AND MOD(CAST(id AS BIGINT), {REDELIVERY_MODULUS}) = 0",
    )


def smelt_run_window(window_start: date, window_end: date) -> None:
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


def smelt_test() -> None:
    run_or_die(["smelt", "test"], cwd=EXAMPLE_DIR)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--start-date",
        type=lambda s: date.fromisoformat(s),
        default=DEFAULT_START,
    )
    parser.add_argument("--days", type=int, default=DEFAULT_DAYS)
    parser.add_argument("--report", type=Path, default=EXAMPLE_DIR / ".last_run.json")
    args = parser.parse_args()

    target_dir = EXAMPLE_DIR / "target"
    target_dir.mkdir(exist_ok=True)
    db = target_dir / "dev.duckdb"
    if db.exists():
        db.unlink()
    setup_sources(db)

    days = daterange(args.start_date, args.days)
    iter_reports: list[IterReport] = []
    total_redelivered = 0
    loop_t0 = time.monotonic()
    for idx, day in enumerate(days, start=1):
        t0 = time.monotonic()
        smelt_run_window(day, day + timedelta(days=1))
        elapsed = time.monotonic() - t0
        redelivered = redelivered_count(db, day)
        total_redelivered += redelivered
        print(
            f"[day {idx:>2}/{len(days)}] {day.isoformat()}  "
            f"redelivered={redelivered}  smelt run {elapsed:.2f}s"
        )
        iter_reports.append(IterReport(idx, len(days), day, redelivered, elapsed))

    loop_seconds = time.monotonic() - loop_t0
    test_t0 = time.monotonic()
    smelt_test()
    test_seconds = time.monotonic() - test_t0

    report = {
        "start_date": args.start_date.isoformat(),
        "days": args.days,
        "total_redelivered_rows": total_redelivered,
        "loop_seconds": loop_seconds,
        "tests_seconds": test_seconds,
        "iterations": [
            {"iter": r.iter_n, "day": r.day.isoformat(), "redelivered": r.redelivered_rows, "seconds": r.seconds}
            for r in iter_reports
        ],
    }
    args.report.write_text(json.dumps(report, indent=2))

    print()
    print("=== summary ===")
    print(f"  {args.days} days replayed in {loop_seconds:.1f}s")
    print(f"  total redelivered rows observed: {total_redelivered}")
    print(f"  report: {args.report}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
