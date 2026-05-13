-- Intentional error: spread operator in a WHERE clause, which is a forbidden
-- position (no comma-separated grammar in WHERE; use the `and_all` reducer
-- from Phase B instead).  The parser error-recovers and ejects the ... token;
-- the type-checker detects the orphaned DOT_DOT_DOT and emits the diagnostic.
-- Emits: MetaSpreadInForbiddenPosition
SELECT
    id
FROM smelt.sources.raw.users
WHERE id = 1 AND ...preds
