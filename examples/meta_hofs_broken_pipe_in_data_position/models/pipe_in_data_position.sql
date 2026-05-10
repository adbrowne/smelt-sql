-- Intentional error: the pipe operator |> is meta-only and must not appear
-- inside a Data-World grammar position such as a WHERE clause.
-- `[1, 2, 3] |> filter(fn x => x > 1)` in a WHERE clause is invalid.
-- Emits: PipeInDataPosition
SELECT 1
WHERE [1, 2, 3] |> filter(fn x => x > 1)
