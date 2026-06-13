-- Clean fixture: uses COLLATE "C" which is a binary (portable) collation.
-- Expected: zero diagnostics.
SELECT name COLLATE "C" AS sorted_name
FROM (SELECT 'example' AS name) AS t
