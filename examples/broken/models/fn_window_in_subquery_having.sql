-- Phase 49 broken fixture: a scalar subquery in the HAVING clause contains a
-- window function.  HAVING is a scalar context; any window function inside the
-- scalar subquery is invalid and must be rejected with `WindowInScalarContext`.
SELECT col, COUNT(*) AS cnt
FROM smelt.models.events
GROUP BY col
HAVING COUNT(*) > (SELECT AVG(RANK() OVER (ORDER BY col)) FROM smelt.models.events)
