"""Validate large-fixture outputs against acceptance criteria.

Checks:
  1. The union table exists and has 9 rows (3 per cohort × 3 cohorts shipped).
  2. Structural: a generator file (models/cohorts.gen.sql or similar) with
     `generates: models` frontmatter exists.
  3. Structural: a YAML config file listing cohort entries exists under configs/.

Connects to the first .duckdb file in the project root, writes eval.json,
and exits non-zero if any check fails.
"""
from __future__ import annotations

import glob
import json
import os
import re
import sys
from pathlib import Path

PROJECT_DIR = Path(os.environ.get("SMELT_PROJECT_DIR", ".")).resolve()
EVAL_PATH = Path(os.environ.get("SMELT_EVAL_OUT", PROJECT_DIR / "eval.json")).resolve()


def find_db() -> Path:
    candidates = sorted(glob.glob(str(PROJECT_DIR / "*.duckdb")))
    if not candidates:
        print(f"FATAL: no .duckdb file found in {PROJECT_DIR}", file=sys.stderr)
        sys.exit(2)
    return Path(candidates[0])


def main() -> int:
    db_path = find_db()
    results: list[dict] = []

    try:
        import duckdb
        con = duckdb.connect(str(db_path), read_only=True)
    except Exception as e:
        results.append({"name": "open duckdb file", "ok": False, "detail": f"{db_path}: {e}"})
        _write_eval(results, db_path)
        print(f"  [FAIL] open duckdb file  {db_path}: {e}")
        return 1

    def check(name: str, ok: bool, detail: str = "") -> None:
        results.append({"name": name, "ok": bool(ok), "detail": detail})
        status = "PASS" if ok else "FAIL"
        print(f"  [{status}] {name}  {detail}")

    def safe_query(sql: str):
        try:
            return con.execute(sql).fetchall()
        except Exception as e:
            return e

    # --- Find the union table (may be named all_orders, all_cohorts, etc.) ----
    print("\n=== union table check ===")
    # Try common names; accept the first one that exists.
    union_table = None
    for candidate in ["all_orders", "all_cohorts", "all_cohorts_unioned"]:
        rows = safe_query(f"SELECT COUNT(*) FROM {candidate}")
        if not isinstance(rows, Exception):
            union_table = candidate
            break

    if union_table is None:
        check("union table exists (all_orders / all_cohorts)", False,
              "no recognised union table found in the database")
    else:
        count = safe_query(f"SELECT COUNT(*) FROM {union_table}")[0][0]
        check(f"union table '{union_table}' exists", True)
        check(
            f"union table has 9 rows (3 shipped per cohort × 3 cohorts)",
            count == 9,
            f"got {count}",
        )

    # --- Acceptance: row-count parity check -----------------------------------
    print("\n=== acceptance: row-count parity ===")
    if union_table is not None:
        # Count rows per country in the union and compare to seed expectations.
        rows = safe_query(
            f"SELECT country, COUNT(*) AS cnt FROM {union_table} GROUP BY country ORDER BY country"
        )
        if isinstance(rows, Exception):
            check("per-country row count query", False, str(rows))
        else:
            country_counts = {row[0]: row[1] for row in rows}
            for country, expected in [("DE", 3), ("UK", 3), ("US", 3)]:
                actual = country_counts.get(country, 0)
                check(
                    f"{country} cohort has {expected} shipped rows",
                    actual == expected,
                    f"got {actual}",
                )

    # --- Structural: generator file exists ------------------------------------
    print("\n=== structural checks ===")
    model_dir = PROJECT_DIR / "models"
    gen_files = list(model_dir.rglob("*.gen.sql")) if model_dir.is_dir() else []
    # Also accept non-.gen.sql files that have `generates: models` frontmatter.
    generates_pattern = re.compile(r"generates\s*:\s*models", re.IGNORECASE)
    all_sql = list(model_dir.rglob("*.sql")) if model_dir.is_dir() else []
    gen_files_by_frontmatter = [
        f for f in all_sql if generates_pattern.search(f.read_text())
    ]
    any_gen = len(gen_files) > 0 or len(gen_files_by_frontmatter) > 0
    check(
        "generator file with `generates: models` frontmatter exists",
        any_gen,
        f".gen.sql files: {[str(f.relative_to(PROJECT_DIR)) for f in gen_files]}, "
        f"frontmatter matches: {[str(f.relative_to(PROJECT_DIR)) for f in gen_files_by_frontmatter]}",
    )

    # --- Structural: YAML config exists ---------------------------------------
    config_dirs = ["configs", "config"]
    yaml_files = []
    for d in config_dirs:
        config_dir = PROJECT_DIR / d
        if config_dir.is_dir():
            yaml_files.extend(config_dir.rglob("*.yaml"))
            yaml_files.extend(config_dir.rglob("*.yml"))
    check(
        "YAML cohort config file exists under configs/",
        len(yaml_files) > 0,
        f"found: {[str(f.relative_to(PROJECT_DIR)) for f in yaml_files]}",
    )

    # --- Summary --------------------------------------------------------------
    passed = sum(1 for r in results if r["ok"])
    total = len(results)
    _write_eval(results, db_path)

    print(f"\n=== Summary ===")
    print(f"  {passed}/{total} checks passed; eval.json -> {EVAL_PATH}")
    return 0 if passed == total else 1


def _write_eval(results: list[dict], db_path: Path) -> None:
    passed = sum(1 for r in results if r["ok"])
    total = len(results)
    summary = {
        "passed": passed,
        "failed": total - passed,
        "total": total,
        "failures": [r for r in results if not r["ok"]],
        "db_path": str(db_path),
    }
    EVAL_PATH.parent.mkdir(parents=True, exist_ok=True)
    EVAL_PATH.write_text(json.dumps(summary, indent=2, default=str))


if __name__ == "__main__":
    sys.exit(main())
