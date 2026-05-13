-- Happy path: spread a list literal into the SELECT list.
-- [name, email] is a List<Text> meta-literal; ...xs expands it into two
-- SELECT items.  After expansion the effective query is:
--   SELECT id, name, email FROM smelt.sources.raw.users
SELECT
    id,
    ...[name, email]
FROM smelt.sources.raw.users
