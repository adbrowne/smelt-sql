-- Intentional error: the right-hand side of |> must be a function call
-- (e.g. filter(...) or map(...)), not an arithmetic expression.
-- `[1, 2, 3] |> 3 + 4` has a non-call RHS.
-- Emits: PipeRhsNotCall
SELECT [1, 2, 3] |> 3 + 4
