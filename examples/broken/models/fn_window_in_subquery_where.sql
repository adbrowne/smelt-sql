-- Phase 49 broken fixture: a scalar subquery in the WHERE clause contains a
-- window function.  The outer WHERE is a scalar context; any window function
-- inside the scalar subquery is invalid and must be rejected with
-- `WindowInScalarContext`.
SELECT col
FROM smelt.ref('events')
WHERE col > (SELECT ROW_NUMBER() OVER (ORDER BY col) FROM smelt.ref('events'))
