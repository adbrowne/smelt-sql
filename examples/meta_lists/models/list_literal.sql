-- Happy path: a homogeneous list literal spread into SELECT.
-- [1, 2, 3] is a List<Integer> meta-literal; ...xs expands it into three
-- SELECT items.  After expansion the effective query is:
--   SELECT id, 1, 2, 3 FROM smelt.sources.raw.users
--
-- Note: nested-list type inference (List<List<T>>) is exercised by the unit
-- tests in Phase 2 of the meta-language plan.  This model exercises a flat
-- List<Integer> literal to keep the fixture straightforward.
SELECT
    id,
    ...[1, 2, 3]
FROM smelt.sources.raw.users
