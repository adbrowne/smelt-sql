-- Model referencing a column that does not exist upstream
SELECT
    user_id,
    nonexistent_col
FROM smelt.models.staging.stg_users

