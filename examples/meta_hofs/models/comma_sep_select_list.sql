-- Happy path: reduce a list of integer expressions with `plus_chain`.
-- reduce([1, 2, 3], plus_chain) folds left with +: 1 + 2 + 3 = 6.
-- Uses a numeric list to avoid column-type inference issues and to exercise
-- the closed reducer registry in a SELECT item context.
-- Aliased so the build-path type-cast wrapper has a stable column name.
SELECT reduce([1, 2, 3], plus_chain) AS total
