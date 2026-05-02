-- Model with a type mismatch: comparing INTEGER user_id to a string
SELECT
    user_id,
    email
FROM smelt.models.staging.stg_users
WHERE user_id = 'abc'

