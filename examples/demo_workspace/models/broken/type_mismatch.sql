-- Model with a type mismatch: comparing INTEGER user_id to a string
SELECT
    user_id,
    email
FROM smelt.ref('stg_users')
WHERE user_id = 'abc'
