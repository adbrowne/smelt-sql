-- Intentional circular dependency: references itself
SELECT *
FROM smelt.models.circular_ref
LIMIT 10
