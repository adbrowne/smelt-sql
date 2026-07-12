# DuckDB differential seed corpus — one statement per line.
#
# Every statement here is valid DuckDB SQL against the schema prelude in
# `src/duckdb_oracle.rs` (table `t(a INTEGER, b VARCHAR, c DOUBLE, d DATE,
# ts TIMESTAMP)`). Lines starting with `#` and blank lines are ignored.
#
# The differential harness (`tests/duckdb_differential.rs`) enforces both
# dialect-conformance directions over this corpus:
#   - accept:   DuckDB accepts ⇒ smelt parses cleanly OR a registered gap.
#   - fidelity: smelt parses cleanly ⇒ printed SQL still executes on DuckDB.
#
# This list seeds the corpus with the constructs the 2026-07-11 parser review
# found smelt mishandles. As grammar support lands, statements move from the
# gap registry to clean-parse + round-trip (the ratchet only shrinks).
SELECT TRY_CAST(b AS INTEGER) AS x FROM t
SELECT a, count(*) AS n FROM t GROUP BY ALL
SELECT a FROM t ORDER BY ALL
SELECT last_value(a IGNORE NULLS) OVER (ORDER BY a) AS x FROM t
SELECT trim(BOTH ' ' FROM b) AS x FROM t
SELECT substring(b FROM 1 FOR 2) AS x FROM t
SELECT position('x' IN b) AS x FROM t
SELECT position(CAST(a = 1 AS VARCHAR) IN b) AS x FROM t
SELECT overlay(b PLACING 'x' FROM 1 FOR 2) AS x FROM t
SELECT a FROM t WHERE b LIKE ANY (['a%', 'b%'])
SELECT $$hello$$ AS x
SELECT [x * 2 FOR x IN [1, 2, 3]] AS l
SELECT MAP {'a': 1, 'b': 2} AS m
SELECT a FROM t WHERE b GLOB 'x*'
SELECT 0x1F AS x
SELECT 1_000_000 AS x
SELECT E'\n' AS x
SELECT B'0101' AS x
SELECT INTERVAL 3 MONTH AS x
SELECT ts AT TIME ZONE 'UTC' AS x FROM t
SELECT CAST(ts AS TIMESTAMPTZ) AT TIME ZONE 'UTC' AS x FROM t
SELECT a, b, SUM(c) FROM t GROUP BY GROUPING SETS ((a), (b), ())
