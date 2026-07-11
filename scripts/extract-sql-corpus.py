#!/usr/bin/env python3
"""Vendor a SELECT-only SQL corpus from DuckDB's sqllogictest suite and
PostgreSQL's regression suite for `smelt-parser-compat`'s external-corpus
conformance gate.

This script is a **one-time / occasionally-rerun** tool. It is NOT invoked by
CI or by `cargo test` — the corpus it produces is vendored (committed) under
`crates/smelt-parser-compat/tests/corpus/external/`. Re-run it by hand when
you want to refresh the corpus against a newer upstream tag.

Usage
-----
    # Regenerate the vendored corpus (downloads ~130MB of upstream source
    # tarballs to a temp dir, extracts only the test-SQL subtrees, deletes
    # the temp dir when done):
    python3 scripts/extract-sql-corpus.py

    # Run the self-tests for the filter/normalize/dedup logic (used by
    # `crates/smelt-parser-compat/tests/external_corpus.rs` so the filter
    # behavior has one source of truth instead of a second, driftable copy
    # written in Rust):
    python3 scripts/extract-sql-corpus.py --self-test

Sources (pinned tags — bump deliberately, not automatically)
--------------------------------------------------------------------------
- DuckDB   `v1.5.0`      — MIT License. `test/sql/**/*.test` sqllogictest
  files; `statement ok` / `query <types>` blocks.
- PostgreSQL `REL_16_4`  — The PostgreSQL License (permissive, similar to
  MIT/BSD). `src/test/regress/sql/*.sql` regression-test scripts.

Both licenses permit vendoring short extracted fragments with attribution;
see the generated `tests/corpus/external/README.md` for the full notices.

Filter
------
Only statements that look like read-only queries are kept:
`SELECT`, `WITH`, or `VALUES` as the first keyword, and no top-level
DDL/DML keyword (CREATE, INSERT, UPDATE, DELETE, DROP, ALTER, COPY,
TRUNCATE, GRANT, REVOKE, VACUUM, EXPLAIN, BEGIN, COMMIT, ROLLBACK, SET,
SHOW) appearing in the statement body — this also excludes data-modifying
CTEs (`WITH x AS (INSERT ... RETURNING ...) ...`). Statements are then
normalized (whitespace-collapsed to a single line, trailing `;` stripped),
deduplicated, and capped to a bounded sample size so the differential
harness that runs them against a real DuckDB stays fast.
"""

from __future__ import annotations

import random
import re
import sys
import tarfile
import tempfile
import urllib.request
from pathlib import Path

DUCKDB_TAG = "v1.5.0"
POSTGRES_TAG = "REL_16_4"

DUCKDB_TARBALL_URL = (
    f"https://codeload.github.com/duckdb/duckdb/tar.gz/refs/tags/{DUCKDB_TAG}"
)
POSTGRES_TARBALL_URL = (
    f"https://codeload.github.com/postgres/postgres/tar.gz/refs/tags/{POSTGRES_TAG}"
)

# Bounded so `cargo test -p smelt-parser-compat` (which runs every kept
# statement through the smelt parser + a real in-memory DuckDB) stays under
# ~60s. See crates/smelt-parser-compat/CLAUDE.md for the measured wall time.
CAP_PER_SOURCE = 750

REPO_ROOT = Path(__file__).resolve().parent.parent
OUTPUT_DIR = REPO_ROOT / "crates" / "smelt-parser-compat" / "tests" / "corpus" / "external"

_LEADING_KEYWORD_RE = re.compile(r"^\s*(SELECT|WITH|VALUES)\b", re.IGNORECASE)
_DISALLOWED_RE = re.compile(
    r"\b(CREATE|INSERT|UPDATE|DELETE|DROP|ALTER|COPY|TRUNCATE|GRANT|REVOKE|"
    r"VACUUM|EXPLAIN|BEGIN|COMMIT|ROLLBACK|SET|SHOW|PREPARE|EXECUTE|DECLARE|"
    r"LISTEN|NOTIFY|LOCK|CALL|MERGE|ATTACH|DETACH|PRAGMA|LOAD|INSTALL|EXPORT|IMPORT)\b",
    re.IGNORECASE,
)
_WHITESPACE_RE = re.compile(r"\s+")


def is_select_only(stmt: str) -> bool:
    """True if `stmt` looks like a read-only SELECT/WITH/VALUES query with no
    top-level DDL/DML keyword (including inside a data-modifying CTE)."""
    stmt = stmt.strip()
    if not stmt:
        return False
    if not _LEADING_KEYWORD_RE.match(stmt):
        return False
    if _DISALLOWED_RE.search(stmt):
        return False
    return True


def normalize(stmt: str) -> str:
    """Collapse a (possibly multi-line) statement to a single normalized
    line: whitespace-collapsed, trailing `;` and surrounding whitespace
    stripped."""
    stmt = stmt.strip()
    if stmt.endswith(";"):
        stmt = stmt[:-1]
    stmt = _WHITESPACE_RE.sub(" ", stmt).strip()
    return stmt


def dedup_and_cap(stmts: list[str], cap: int, seed: int = 42) -> list[str]:
    """Deduplicate (order-preserving) then, if over `cap`, take a
    reproducible random sample so the corpus stays a representative slice
    of the source rather than just "whatever came first alphabetically"."""
    seen: set[str] = set()
    deduped: list[str] = []
    for s in stmts:
        if s not in seen:
            seen.add(s)
            deduped.append(s)
    if len(deduped) <= cap:
        return deduped
    rng = random.Random(seed)
    sample = rng.sample(deduped, cap)
    # Keep deterministic (source) order rather than shuffled order.
    order = {s: i for i, s in enumerate(deduped)}
    sample.sort(key=lambda s: order[s])
    return sample


# --------------------------------------------------------------------------
# DuckDB sqllogictest extraction
# --------------------------------------------------------------------------

_BLOCK_START_RE = re.compile(r"^(statement\s+ok|query\s+\S*)\s*$", re.IGNORECASE)


def extract_duckdb_statements_from_text(text: str) -> list[str]:
    """Extract the SQL body of every `statement ok` / `query <types>` block
    in one sqllogictest `.test` file's contents."""
    out: list[str] = []
    lines = text.splitlines()
    i = 0
    n = len(lines)
    while i < n:
        line = lines[i]
        if _BLOCK_START_RE.match(line.strip()):
            i += 1
            body: list[str] = []
            while i < n:
                cur = lines[i]
                stripped = cur.strip()
                if stripped == "" or stripped == "----":
                    break
                body.append(cur)
                i += 1
            if body:
                out.append("\n".join(body))
        else:
            i += 1
    return out


def extract_duckdb_corpus(duckdb_src_root: Path) -> list[str]:
    stmts: list[str] = []
    for test_file in sorted((duckdb_src_root / "test" / "sql").rglob("*.test")):
        try:
            text = test_file.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        stmts.extend(extract_duckdb_statements_from_text(text))
    return stmts


# --------------------------------------------------------------------------
# PostgreSQL regression-suite extraction
# --------------------------------------------------------------------------


def _strip_pg_comments(text: str) -> str:
    """Strip `--` line comments (simple, line-oriented — the regress `.sql`
    fixtures don't rely on `--` inside string literals in ways that would
    make this unsafe for a best-effort extraction script)."""
    out_lines = []
    for line in text.splitlines():
        idx = line.find("--")
        out_lines.append(line[:idx] if idx != -1 else line)
    return "\n".join(out_lines)


def extract_postgres_statements_from_text(text: str) -> list[str]:
    """Split one regress `.sql` file into `;`-terminated statements (naive
    top-level split — sufficient for a best-effort extraction pass; anything
    that splits incorrectly simply fails the SELECT-only filter or fails to
    parse and is either dropped or lands in the failure ledger)."""
    text = _strip_pg_comments(text)
    parts = [p.strip() for p in text.split(";")]
    return [p for p in parts if p]


def extract_postgres_corpus(pg_src_root: Path) -> list[str]:
    stmts: list[str] = []
    regress_sql = pg_src_root / "src" / "test" / "regress" / "sql"
    for sql_file in sorted(regress_sql.glob("*.sql")):
        try:
            text = sql_file.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        stmts.extend(extract_postgres_statements_from_text(text))
    return stmts


# --------------------------------------------------------------------------
# Download + extract helpers
# --------------------------------------------------------------------------


def _download_and_extract(url: str, member_prefix_suffix: str, dest: Path) -> Path:
    """Download `url` (a GitHub codeload tarball) and extract only members
    whose path contains `member_prefix_suffix` into `dest`. Returns the path
    to the extracted top-level directory (the tag-suffixed repo root)."""
    print(f"downloading {url} ...", file=sys.stderr)
    with tempfile.NamedTemporaryFile(suffix=".tar.gz") as tmp:
        urllib.request.urlretrieve(url, tmp.name)  # noqa: S310 - pinned https URL
        with tarfile.open(tmp.name, "r:gz") as tf:
            members = [
                m for m in tf.getmembers() if member_prefix_suffix in m.name
            ]
            tf.extractall(dest, members=members)  # noqa: S202 - filtered members only
    # The tarball's single top-level directory is `<repo>-<tag-without-v>`.
    top_level = next(dest.iterdir())
    return top_level


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="smelt-sql-corpus-") as tmp_str:
        tmp = Path(tmp_str)

        duckdb_root = _download_and_extract(
            DUCKDB_TARBALL_URL, "/test/sql/", tmp / "duckdb"
        )
        raw_duckdb = extract_duckdb_corpus(duckdb_root)
        print(f"DuckDB: {len(raw_duckdb)} raw statement blocks extracted", file=sys.stderr)

        pg_root = _download_and_extract(
            POSTGRES_TARBALL_URL, "/src/test/regress/sql/", tmp / "postgres"
        )
        raw_pg = extract_postgres_corpus(pg_root)
        print(f"PostgreSQL: {len(raw_pg)} raw statements extracted", file=sys.stderr)

    duckdb_filtered = [normalize(s) for s in raw_duckdb if is_select_only(s)]
    pg_filtered = [normalize(s) for s in raw_pg if is_select_only(s)]

    duckdb_final = dedup_and_cap(duckdb_filtered, CAP_PER_SOURCE)
    pg_final = dedup_and_cap(pg_filtered, CAP_PER_SOURCE)

    print(
        f"DuckDB: {len(raw_duckdb)} raw -> {len(duckdb_filtered)} SELECT-only "
        f"-> {len(duckdb_final)} after dedup+cap",
        file=sys.stderr,
    )
    print(
        f"PostgreSQL: {len(raw_pg)} raw -> {len(pg_filtered)} SELECT-only "
        f"-> {len(pg_final)} after dedup+cap",
        file=sys.stderr,
    )

    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    _write_corpus_file(OUTPUT_DIR / "duckdb.sql", duckdb_final, "DuckDB", DUCKDB_TAG)
    _write_corpus_file(OUTPUT_DIR / "postgres.sql", pg_final, "PostgreSQL", POSTGRES_TAG)
    _write_readme(OUTPUT_DIR / "README.md", len(duckdb_final), len(pg_final))

    print(f"Wrote corpus to {OUTPUT_DIR}", file=sys.stderr)
    return 0


def _write_corpus_file(path: Path, stmts: list[str], source: str, tag: str) -> None:
    lines = [
        f"# Vendored from {source} {tag} by scripts/extract-sql-corpus.py.",
        "# One statement per line. Do not hand-edit; re-run the script to refresh.",
        "# See ./README.md for license/attribution notices.",
        "",
    ]
    lines.extend(stmts)
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def _write_readme(path: Path, duckdb_count: int, pg_count: int) -> None:
    path.write_text(
        f"""# External SQL corpus — attribution and license notices

This directory vendors a filtered, SELECT-only subset of SQL statements
extracted from two upstream test suites by `scripts/extract-sql-corpus.py`.
The extraction script is documented and re-runnable; it is **not** run in
CI — the extracted files below are committed and used directly by
`crates/smelt-parser-compat/tests/external_corpus.rs`.

## DuckDB ({DUCKDB_TAG}) — `duckdb.sql` ({duckdb_count} statements)

Source: https://github.com/duckdb/duckdb, `test/sql/**/*.test` sqllogictest
files (`statement ok` / `query <types>` blocks only).

DuckDB is licensed under the MIT License:

> Copyright 2018-2025 Stichting DuckDB Foundation
>
> Permission is hereby granted, free of charge, to any person obtaining a copy
> of this software and associated documentation files (the "Software"), to
> deal in the Software without restriction, including without limitation the
> rights to use, copy, modify, merge, publish, distribute, sublicense, and/or
> sell copies of the Software, and to permit persons to whom the Software is
> furnished to do so, subject to the following conditions: the above
> copyright notice and this permission notice shall be included in all copies
> or substantial portions of the Software. THE SOFTWARE IS PROVIDED "AS IS",
> WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED.

## PostgreSQL ({POSTGRES_TAG}) — `postgres.sql` ({pg_count} statements)

Source: https://github.com/postgres/postgres,
`src/test/regress/sql/*.sql` regression-test scripts.

PostgreSQL is distributed under the PostgreSQL License, a liberal
MIT/BSD-style license:

> PostgreSQL Database Management System
> (formerly known as Postgres, then as Postgres95)
>
> Portions Copyright (c) 1996-2025, PostgreSQL Global Development Group
> Portions Copyright (c) 1994, The Regents of the University of California
>
> Permission to use, copy, modify, and distribute this software and its
> documentation for any purpose, without fee, and without a written agreement
> is hereby granted, provided that the above copyright notice and this
> paragraph and the following two paragraphs appear in all copies.

## Regenerating

```bash
python3 scripts/extract-sql-corpus.py
```

Bumps the pinned tags by editing `DUCKDB_TAG` / `POSTGRES_TAG` at the top of
the script first. After regenerating, re-run
`cargo test -p smelt-parser-compat --test external_corpus` and triage any
new failures into `../external_ledger.toml`.
""",
        encoding="utf-8",
    )


# --------------------------------------------------------------------------
# Self-test (invoked by the Rust unit test in
# crates/smelt-parser-compat/tests/external_corpus.rs so the filter logic
# has exactly one source of truth instead of a duplicated Rust copy).
# --------------------------------------------------------------------------


def _self_test() -> int:
    failures: list[str] = []

    def check(cond: bool, msg: str) -> None:
        if not cond:
            failures.append(msg)

    check(is_select_only("SELECT 1"), "plain SELECT should pass")
    check(is_select_only("  select a from t"), "lowercase select should pass")
    check(is_select_only("WITH x AS (SELECT 1) SELECT * FROM x"), "WITH CTE should pass")
    check(is_select_only("VALUES (1), (2)"), "VALUES should pass")
    check(not is_select_only("CREATE TABLE t (a INT)"), "CREATE TABLE must be rejected")
    check(not is_select_only("INSERT INTO t VALUES (1)"), "INSERT must be rejected")
    check(not is_select_only("UPDATE t SET a = 1"), "UPDATE must be rejected")
    check(not is_select_only("DELETE FROM t"), "DELETE must be rejected")
    check(not is_select_only(""), "empty statement must be rejected")
    check(
        not is_select_only("WITH x AS (INSERT INTO t VALUES (1) RETURNING a) SELECT * FROM x"),
        "data-modifying CTE must be rejected",
    )
    check(
        not is_select_only("SELECT 1; DROP TABLE t"),
        "trailing DDL after a SELECT must be rejected",
    )

    check(normalize("SELECT   1\n  FROM t;") == "SELECT 1 FROM t", "normalize collapses whitespace + strips semicolon")
    check(normalize("SELECT 1") == "SELECT 1", "normalize is a no-op on an already-clean statement")

    deduped = dedup_and_cap(["a", "b", "a", "c"], cap=10)
    check(deduped == ["a", "b", "c"], f"dedup_and_cap should preserve order and drop dupes, got {deduped}")

    capped = dedup_and_cap([str(i) for i in range(100)], cap=10)
    check(len(capped) == 10, f"dedup_and_cap should cap to the requested size, got {len(capped)}")
    check(len(set(capped)) == 10, "capped sample should have no duplicates")

    blocks = extract_duckdb_statements_from_text(
        "statement ok\nCREATE TABLE t (a INT);\n\nquery I\nSELECT a FROM t\n----\n1\n"
    )
    check(
        blocks == ["CREATE TABLE t (a INT);", "SELECT a FROM t"],
        f"duckdb block extraction mismatch: {blocks}",
    )

    pg_stmts = extract_postgres_statements_from_text(
        "-- a comment\nSELECT 1;\nSELECT 2 -- trailing comment\nFROM t;\n"
    )
    check(
        pg_stmts == ["SELECT 1", "SELECT 2 \nFROM t"],
        f"postgres statement split mismatch: {pg_stmts}",
    )

    if failures:
        for f in failures:
            print(f"FAIL: {f}", file=sys.stderr)
        return 1
    print(f"OK: all self-tests passed", file=sys.stderr)
    return 0


if __name__ == "__main__":
    if "--self-test" in sys.argv:
        sys.exit(_self_test())
    sys.exit(main())
