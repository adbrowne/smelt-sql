-- Happy path: pipe-rewrite of a HOF chain.
-- [1, 2, 3] |> filter(fn c => c > 0) |> map(fn c => c * 2)
-- desugars to map(filter([1, 2, 3], fn c => c > 0), fn c => c * 2)
-- Exercises the pure meta-language surface: list literal + pipe + lambdas.
-- No source table reference: check_type_diagnostics early-returns so only
-- the meta-language checks in check_file_diagnostics run.
SELECT [1, 2, 3] |> filter(fn c => c > 0) |> map(fn c => c * 2)
