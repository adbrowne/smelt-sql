-- Happy path: reduce a list of Boolean literals with the `and_all` reducer.
-- reduce([true, false, true], and_all) folds left over the Boolean list using AND,
-- effectively expanding to: true AND false AND true.
-- Uses boolean literals to avoid column-type inference issues and to exercise
-- the reducer with a known-good List<Boolean> input.
SELECT reduce([true, false, true], and_all)
