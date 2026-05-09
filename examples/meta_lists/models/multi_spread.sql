-- Happy path: multiple spreads in one SELECT list.
-- Exercises spreading two separate list literals into the same SELECT.
-- Effective query: SELECT id, name, email, created_at FROM smelt.sources.raw.users
SELECT
    id,
    ...[name, email],
    created_at
FROM smelt.sources.raw.users
