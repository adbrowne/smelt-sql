-- Happy path: spread of an empty list elides itself and adjacent commas.
-- Per spec rule 7: SELECT id, ...[], created_at is equivalent to
--   SELECT id, created_at
-- No diagnostic is emitted; the empty spread is silently dropped.
SELECT
    id,
    ...[],
    created_at
FROM smelt.sources.raw.users
