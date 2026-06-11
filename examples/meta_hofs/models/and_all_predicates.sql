--- name: and_all_predicates ---
---
-- Happy path: reduce a list of Boolean literals with the `and_all` reducer.
-- reduce([true, false, true], and_all) folds left over the Boolean list using AND,
-- effectively expanding to: true AND false AND true.
-- Uses boolean literals to avoid column-type inference issues and to exercise
-- the reducer with a known-good List<Boolean> input.
-- Aliased so the build-path type-cast wrapper has a stable column name.
SELECT reduce([true, false, true], and_all) AS all_true

--- name: reduce_is_false ---
materialization: test
test:
  model: and_all_predicates
  expect:
    - {all_true: false}
---