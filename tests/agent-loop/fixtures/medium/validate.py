"""Validate medium-fixture outputs against acceptance criteria.

Checks:
  1. Output table contents match the small-fixture acceptance criteria.
  2. Structural: a functions/ directory exists with at least one smelt.define.
  3. Structural: at least one model file calls smelt.functions.*.

Connects to the first .duckdb file in the project root, writes eval.json,
and exits non-zero if any check fails.
"""
from __future__ import annotations

import glob
import json
import os
import re
import sys
from decimal import Decimal
from pathlib import Path

import duckdb

PROJECT_DIR = Path(os.environ.get("SMELT_PROJECT_DIR", ".")).resolve()
EVAL_PATH = Path(os.environ.get("SMELT_EVAL_OUT", PROJECT_DIR / "eval.json")).resolve()


def find_db() -> Path:
    candidates = sorted(glob.glob(str(PROJECT_DIR / "*.duckdb")))
    if not candidates:
        print(f"FATAL: no .duckdb file found in {PROJECT_DIR}", file=sys.stderr)
        sys.exit(2)
    return Path(candidates[0])


def approx_eq(a, b, tol: float = 0.01) -> bool:
    try:
        return abs(float(a) - float(b)) <= tol
    except (TypeError, ValueError):
        return False


def main() -> int:
    db_path = find_db()
    results: list[dict] = []

    try:
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

    # --- stg_orders ----------------------------------------------------------
    print("\n=== stg_orders ===")
    rows = safe_query("SELECT COUNT(*) FROM stg_orders")
    if isinstance(rows, Exception):
        check("stg_orders exists", False, str(rows))
    else:
        n = rows[0][0]
        check("stg_orders exists", True)
        check("stg_orders has 10 rows", n == 10, f"got {n}")

    # --- int_orders_by_day ---------------------------------------------------
    print("\n=== int_orders_by_day ===")
    rows = safe_query("SELECT COUNT(*), SUM(total_revenue) FROM int_orders_by_day")
    if isinstance(rows, Exception):
        check("int_orders_by_day exists", False, str(rows))
    else:
        n, total = rows[0]
        check("int_orders_by_day exists", True)
        check("int_orders_by_day has 8 rows (one per shipped day)", n == 8, f"got {n}")
        check(
            "int_orders_by_day total_revenue sums to 890",
            approx_eq(total, 890),
            f"got {total}",
        )

        rows2 = safe_query(
            "SELECT order_count, total_revenue FROM int_orders_by_day "
            "WHERE order_date = DATE '2025-01-01'"
        )
        if isinstance(rows2, Exception) or not rows2:
            check("int_orders_by_day row for 2025-01-01", False, "missing")
        else:
            oc, rev = rows2[0]
            check(
                "int_orders_by_day 2025-01-01 has 2 orders / $150",
                oc == 2 and approx_eq(rev, 150),
                f"got order_count={oc} total_revenue={rev}",
            )

    # --- mart_top_customers --------------------------------------------------
    print("\n=== mart_top_customers ===")
    rows = safe_query(
        "SELECT customer_id, customer_name, total_revenue FROM mart_top_customers "
        "ORDER BY total_revenue DESC, customer_id ASC"
    )
    if isinstance(rows, Exception):
        check("mart_top_customers exists", False, str(rows))
    else:
        check("mart_top_customers exists", True)
        check(
            "mart_top_customers has 5 rows (one per customer)",
            len(rows) == 5,
            f"got {len(rows)}",
        )
        if rows:
            top_id, top_name, top_rev = rows[0]
            check(
                "top customer is customer_id=1 with $490 revenue",
                top_id == 1 and approx_eq(top_rev, 490),
                f"got customer_id={top_id} name={top_name!r} revenue={top_rev}",
            )
            mart_total = sum(Decimal(str(r[2])) for r in rows)
            check(
                "mart_top_customers total revenue across all rows = 890",
                approx_eq(mart_total, 890),
                f"got {mart_total}",
            )

    # --- structural: functions directory -------------------------------------
    print("\n=== structural checks ===")
    fn_dir = PROJECT_DIR / "functions"
    fn_sql_files = list(fn_dir.rglob("*.sql")) if fn_dir.is_dir() else []
    check("functions/ directory exists", fn_dir.is_dir())

    define_pattern = re.compile(r"\bsmelt\.define\b", re.IGNORECASE)
    files_with_define = [f for f in fn_sql_files if define_pattern.search(f.read_text())]
    check(
        "at least one smelt.define in functions/",
        len(files_with_define) > 0,
        f"found in: {[str(f.relative_to(PROJECT_DIR)) for f in files_with_define]}",
    )

    typed_param_pattern = re.compile(r":\s*Expr\s*<", re.IGNORECASE)
    files_with_types = [f for f in files_with_define if typed_param_pattern.search(f.read_text())]
    check(
        "at least one smelt.define has typed parameters (Expr<T>)",
        len(files_with_types) > 0,
        f"found in: {[str(f.relative_to(PROJECT_DIR)) for f in files_with_types]}",
    )

    call_pattern = re.compile(r"\bsmelt\.functions\.", re.IGNORECASE)
    model_dir = PROJECT_DIR / "models"
    model_files = list(model_dir.rglob("*.sql")) if model_dir.is_dir() else []
    models_calling_fn = [f for f in model_files if call_pattern.search(f.read_text())]
    check(
        "at least one model calls smelt.functions.*",
        len(models_calling_fn) > 0,
        f"found in: {[str(f.relative_to(PROJECT_DIR)) for f in models_calling_fn]}",
    )

    # --- summary -------------------------------------------------------------
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
