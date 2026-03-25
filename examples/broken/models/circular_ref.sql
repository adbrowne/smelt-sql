-- Intentional circular dependency: references itself
SELECT *
FROM smelt.ref('circular_ref')
LIMIT 10
