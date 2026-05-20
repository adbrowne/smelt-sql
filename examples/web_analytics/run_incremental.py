#!/usr/bin/env python3
"""Datagen + day-by-day incremental replay driver for examples/web_analytics.

What this script does, in order:
  1. Wipe target/dev.duckdb so the run is reproducible.
  2. Invoke smelt-datagen at the requested scale factor (default 0.01)
     to write partitioned Parquet under data/.
  3. Materialise the raw.* source tables in target/dev.duckdb via
     setup_sources.sql (DuckDB CLI).
  4. Loop day-by-day across the datagen window, invoking
        smelt run --event-time-start D-1 --event-time-end D+1
     once per day.  The 2-day window honours the 1-day lookback that
     gold/identity_forward_only and silver/sessions need to catch
     cross-midnight sessions / late-arriving signins.
  5. Finish with `smelt test` so all inline invariants are exercised
     against the final cumulative state.

Per-iteration output is one structured line of the shape
    [day N/total] YYYY-MM-DD  smelt run [prev=YYYY-MM-DD next=YYYY-MM-DD]  W.Xs
plus a final summary block.  Non-zero exit on any subprocess failure.

Requires: smelt-datagen and smelt on PATH; duckdb on PATH; Python 3.9+.
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

# Defaults reflect datagen.yaml: events partition starts 2026-03-19 and
# spans 60 days, so dates are [2026-03-19, 2026-05-18).
DEFAULT_START = date(2026, 3, 19)
DEFAULT_DAYS = 60
DEFAULT_SCALE = 0.01


@dataclass
class IterReport:
    iter_n: int
    total: int
    day: date
    window_start: date
    window_end: date
    seconds: float


def daterange(start: date, days: int) -> list[date]:
    return [start + timedelta(days=i) for i in range(days)]


def run_or_die(cmd: list[str], *, cwd: Path | None = None) -> None:
    """Run a subprocess and exit non-zero on failure, surfacing its output."""
    result = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True)
    if result.returncode != 0:
        sys.stderr.write(f"\n[FAIL] command exited {result.returncode}\n")
        sys.stderr.write(f"  cmd: {' '.join(cmd)}\n")
        if result.stdout:
            sys.stderr.write(f"  stdout:\n{result.stdout}\n")
        if result.stderr:
            sys.stderr.write(f"  stderr:\n{result.stderr}\n")
        sys.exit(result.returncode)


def phase(label: str, fn) -> float:
    """Run `fn()`, print a [phase] timing line, return wall-clock seconds."""
    t0 = time.monotonic()
    fn()
    elapsed = time.monotonic() - t0
    print(f"[{label}] {elapsed:.1f}s")
    return elapsed


def datagen(scale_factor: float) -> None:
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
        sys.stderr.write(f"\n[FAIL] duckdb < setup_sources.sql exited {proc.returncode}\n")
        sys.stderr.write(f"  stderr:\n{proc.stderr}\n")
        sys.exit(proc.returncode)


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
        help=f"first event_date to process (default: {DEFAULT_START.isoformat()})",
    )
    parser.add_argument(
        "--days",
        type=int,
        default=DEFAULT_DAYS,
        help=f"number of consecutive days to process (default: {DEFAULT_DAYS})",
    )
    parser.add_argument(
        "--scale-factor",
        type=float,
        default=DEFAULT_SCALE,
        help=f"smelt-datagen scale factor (default: {DEFAULT_SCALE})",
    )
    parser.add_argument(
        "--skip-datagen",
        action="store_true",
        help="reuse existing data/ Parquet and target/dev.duckdb (skips datagen + setup_sources.sql)",
    )
    parser.add_argument(
        "--report",
        type=Path,
        default=EXAMPLE_DIR / ".last_run.json",
        help="path to write the per-iteration timing report (default: .last_run.json)",
    )
    args = parser.parse_args()

    if not args.skip_datagen:
        target_db = EXAMPLE_DIR / "target" / "dev.duckdb"
        if target_db.exists():
            target_db.unlink()
        (EXAMPLE_DIR / "target").mkdir(exist_ok=True)
        data_dir = EXAMPLE_DIR / "data"
        if data_dir.exists():
            shutil.rmtree(data_dir)
        datagen_seconds = phase("datagen", lambda: datagen(args.scale_factor))
        setup_seconds = phase("setup", setup_sources)
    else:
        datagen_seconds = 0.0
        setup_seconds = 0.0
        print("[skip-datagen] reusing existing data/ and target/dev.duckdb")

    days = daterange(args.start_date, args.days)
    iter_reports: list[IterReport] = []
    loop_t0 = time.monotonic()
    for idx, day in enumerate(days, start=1):
        window_start = day - timedelta(days=1)
        window_end = day + timedelta(days=1)
        t0 = time.monotonic()
        smelt_run_window(window_start, window_end)
        elapsed = time.monotonic() - t0
        print(
            f"[day {idx:>2}/{len(days)}] {day.isoformat()}  "
            f"smelt run [prev={window_start.isoformat()} next={window_end.isoformat()}]  "
            f"{elapsed:.1f}s"
        )
        iter_reports.append(
            IterReport(
                iter_n=idx,
                total=len(days),
                day=day,
                window_start=window_start,
                window_end=window_end,
                seconds=elapsed,
            )
        )

    loop_seconds = time.monotonic() - loop_t0
    test_seconds = phase("tests", smelt_test)

    report = {
        "start_date": args.start_date.isoformat(),
        "days": args.days,
        "scale_factor": args.scale_factor,
        "datagen_seconds": datagen_seconds,
        "setup_seconds": setup_seconds,
        "loop_seconds": loop_seconds,
        "tests_seconds": test_seconds,
        "total_iterations": len(iter_reports),
        "iterations": [
            {
                "iter": r.iter_n,
                "day": r.day.isoformat(),
                "window_start": r.window_start.isoformat(),
                "window_end": r.window_end.isoformat(),
                "seconds": r.seconds,
            }
            for r in iter_reports
        ],
    }
    args.report.write_text(json.dumps(report, indent=2))

    total = datagen_seconds + setup_seconds + loop_seconds + test_seconds
    print()
    print(f"=== summary ===")
    print(f"  {args.days} days replayed in {loop_seconds:.1f}s ({loop_seconds / max(args.days, 1):.2f}s/day)")
    print(f"  total wall-clock: {total:.1f}s")
    print(f"  report: {args.report}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
