-- Intentional error: references a source that does not exist
SELECT *
FROM smelt.source('nonexistent_database.missing_table')
