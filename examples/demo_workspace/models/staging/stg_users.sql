-- Cleaned user data from raw source
SELECT
    user_id,
    LOWER(email) AS email,
    signup_date,
    plan_type
FROM smelt.source('raw.users')
