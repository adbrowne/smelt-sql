-- Clean fixture: binary-collation string GROUP BY, DISTINCT, and ORDER BY.
-- All operations use the default binary collation (no COLLATE clause needed)
-- or explicit binary COLLATE "C", both of which are portable.
-- Expected: zero diagnostics.
SELECT
    name,
    COUNT(*) AS name_count
FROM (
    SELECT 'alice' AS name
    UNION ALL
    SELECT 'bob'
    UNION ALL
    SELECT 'alice'
) AS t
GROUP BY name
ORDER BY name
