-- Fixture for the statement-level restructure planner
-- (`docs/specs/multi_backend.md` §"Statement-level lowering"): an
-- analytic-only built-in reached under `GROUP BY`. GoogleSQL rejects
-- `PERCENTILE_CONT ... WITHIN GROUP` outright and requires an `OVER`
-- clause, so this shape restructures the `FROM`/`WHERE` into a CTE that
-- adds the value as an analytic column over the grouping keys.
SELECT
    g,
    COUNT(*) AS n,
    PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) AS med
FROM (VALUES
    (1, 10.0),
    (1, 20.0),
    (1, 30.0),
    (2, 5.0),
    (2, 15.0)
) AS t(g, x)
GROUP BY g
