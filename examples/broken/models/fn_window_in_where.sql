-- Phase 14 broken fixture (§16 #24): a SELECT whose WHERE clause contains a
-- window function call. The kind ceiling at the WHERE splice point is
-- `Scalar`, so a `Window`-kind expression must be rejected with
-- `WindowInScalarContext`. The diagnostic anchors at the offending
-- `ROW_NUMBER() OVER (...) > 1` expression.
SELECT *
FROM smelt.ref('events')
WHERE ROW_NUMBER() OVER (ORDER BY occurred_at) > 1
