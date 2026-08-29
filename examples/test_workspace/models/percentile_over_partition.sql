-- Fixture for the statement-level restructure planner
-- (`docs/specs/multi_backend.md` §"Statement-level lowering"): an
-- aggregate-only built-in reached with a whole-partition `OVER` clause.
-- DuckDB and Spark have the ordered-set `PERCENTILE_CONT` aggregate but no
-- window form of it, so this shape binds the source once, groups it by the
-- partition keys, and joins the grouped result back.
SELECT
    id,
    g,
    PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) OVER (PARTITION BY g) AS med
FROM (VALUES
    (1, 1, 10.0),
    (2, 1, 20.0),
    (3, 1, 30.0),
    (4, 2, 5.0),
    (5, 2, 15.0)
) AS t(id, g, x)
