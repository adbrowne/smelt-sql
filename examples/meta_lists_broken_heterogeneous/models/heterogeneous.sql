-- Intentional error: list literal with heterogeneous element types.
-- [1, 'hello'] cannot unify: the first element is Integer, the second is Text.
-- Emits: MetaListHeterogeneous
SELECT
    id,
    ...[1, 'hello']
FROM smelt.sources.raw.users
