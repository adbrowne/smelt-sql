#!/usr/bin/env python3
"""Incremental replay driver for examples/github_activity.

Unlike `examples/web_analytics/run_incremental.py`, there is no datagen step:
`seeds/github_events_sample.parquet` is a committed, reproducible export of
`sample.sql` (`refresh_sample.sh` regenerates it against BigQuery). What this
script does, in order:

  1. Wipe target/dev.duckdb and create the empty `raw.github_events` table
     via setup_sources.sql.
  2. Loop window-by-window across the fixture's 30-day range, invoking `smelt
     run --event-time-start W --event-time-end W+width`. The window is one day
     wide by default — the fixture's natural cadence — and `--window-days`
     widens it, because the CLI's range is a run window rather than a
     per-partition invocation (`docs/specs/incremental_shapes.md` §"Run window
     vs partition granularity"). Note that a window wider than the loader's
     `cadence: 1 day` invokes `load_day.sh` once, for the window's FIRST day
     only (`{run_date}` is the run-window start), so a coarse schedule wants
     `--preload-source` to stage the source up front. The day's load is no
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
import shutil
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


def smelt_run_window(
    window_start: date,
    window_end: date,
    exclude: list[str] | None = None,
    full_refresh: bool = False,
) -> None:
    cmd = [
        "smelt",
        "run",
        "--event-time-start",
        window_start.isoformat(),
        "--event-time-end",
        window_end.isoformat(),
    ]
    if full_refresh:
        cmd.append("--full-refresh")
    for model in exclude or []:
        cmd += ["-e", model]
    run_or_die(cmd, cwd=EXAMPLE_DIR)


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
    parser.add_argument(
        "--window-days",
        type=int,
        default=1,
        help="width of each `smelt run` window in days (default 1, the fixture's natural "
        "daily cadence). The CLI's [--event-time-start, --event-time-end) range is a run "
        "window, not a per-partition invocation (`docs/specs/incremental_shapes.md` "
        "\u00a7\"Run window vs partition granularity\"), so the same --days of fixture can be "
        "replayed as thirty daily windows or six five-day ones. A width that does not "
        "divide --days leaves a short final window rather than dropping days.",
    )
    parser.add_argument(
        "--first-full-refresh",
        action="store_true",
        help="run the first window with --full-refresh (the rest stay incremental)",
    )
    parser.add_argument("--report", type=Path, default=EXAMPLE_DIR / ".last_run.json")
    parser.add_argument(
        "--exclude",
        action="append",
        default=[],
        metavar="MODEL",
        help="passed through to `smelt run -e` on every window (repeatable)",
    )
    parser.add_argument(
        "--snapshot-dir",
        type=Path,
        help="copy the DuckDB file after each --snapshot-after window into this directory, "
        "as w<NN>.duckdb",
    )
    parser.add_argument(
        "--snapshot-after",
        default="",
        metavar="N[,N...]",
        help="comma-separated 1-based window numbers to snapshot after",
    )
    parser.add_argument(
        "--preload-source",
        action="store_true",
        help="run load_day.sh for every day in the range BEFORE the first window, so the "
        "windows advance over a fully-populated source rather than one that grows a day "
        "at a time. `load_day.sh`'s own per-day idempotence then makes the declared "
        "external step a no-op inside each run. This is the arrival order a "
        "warehouse-resident source has, and isolating it is what lets a cross-target "
        "difference be attributed to arrival order rather than to the engine.",
    )
    parser.add_argument(
        "--skip-tests",
        action="store_true",
        help="skip the closing `smelt test` (it asserts over models --exclude may have "
        "kept out of the run)",
    )
    args = parser.parse_args()
    snapshot_after = {
        int(n) for n in args.snapshot_after.split(",") if n.strip()
    }
    if snapshot_after and args.snapshot_dir is None:
        parser.error("--snapshot-after needs --snapshot-dir")

    target_dir = EXAMPLE_DIR / "target"
    target_dir.mkdir(exist_ok=True)
    db = target_dir / "dev.duckdb"
    if db.exists():
        db.unlink()
    setup_sources(db)

    if args.window_days < 1:
        parser.error("--window-days must be at least 1")
    days = daterange(args.start_date, args.days)
    windows = [
        (
            args.start_date + timedelta(days=i * args.window_days),
            args.start_date + timedelta(days=min((i + 1) * args.window_days, args.days)),
        )
        for i in range((args.days + args.window_days - 1) // args.window_days)
    ]
    if args.preload_source:
        for day in days:
            run_or_die(
                ["bash", str(EXAMPLE_DIR / "load_day.sh"), "--date", day.isoformat()],
                cwd=EXAMPLE_DIR,
            )
        print(f"preloaded {len(days)} day(s) of source before the first window")
    iter_reports: list[IterReport] = []
    total_redelivered = 0
    loop_t0 = time.monotonic()
    for idx, (w_start, w_end) in enumerate(windows, start=1):
        t0 = time.monotonic()
        smelt_run_window(
            w_start,
            w_end,
            args.exclude,
            full_refresh=(idx == 1 and args.first_full_refresh),
        )
        elapsed = time.monotonic() - t0
        if idx in snapshot_after:
            args.snapshot_dir.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(db, args.snapshot_dir / f"w{idx:02d}.duckdb")
        # The redelivery count is a per-day property of the fixture, so a wide
        # window sums the days it covers rather than reporting one of them.
        redelivered = sum(
            redelivered_count(db, w_start + timedelta(days=k))
            for k in range((w_end - w_start).days)
        )
        total_redelivered += redelivered
        print(
            f"[window {idx:>2}/{len(windows)}] [{w_start.isoformat()} .. {w_end.isoformat()})  "
            f"redelivered={redelivered}  smelt run {elapsed:.2f}s"
        )
        iter_reports.append(IterReport(idx, len(windows), w_start, redelivered, elapsed))

    loop_seconds = time.monotonic() - loop_t0
    test_t0 = time.monotonic()
    if not args.skip_tests:
        smelt_test()
    test_seconds = time.monotonic() - test_t0

    report = {
        "start_date": args.start_date.isoformat(),
        "days": args.days,
        "window_days": args.window_days,
        "windows": len(windows),
        "first_full_refresh": args.first_full_refresh,
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
    print(
        f"  {args.days} days replayed as {len(windows)} window(s) of "
        f"{args.window_days} day(s) in {loop_seconds:.1f}s"
    )
    print(f"  total redelivered rows observed: {total_redelivered}")
    print(f"  report: {args.report}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
