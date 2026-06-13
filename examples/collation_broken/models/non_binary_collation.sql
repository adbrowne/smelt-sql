-- Broken fixture: uses COLLATE NOCASE which is a non-binary collation.
-- Expected: exactly one NonPortableCollation diagnostic.
SELECT name COLLATE NOCASE AS sorted_name
FROM (SELECT 'example' AS name) AS t
