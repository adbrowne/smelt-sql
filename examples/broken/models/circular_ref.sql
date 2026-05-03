-- Intentional circular dependency: references itself
SELECT *
FROM smelt.circular_ref
LIMIT 10
