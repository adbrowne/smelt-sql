-- User information from raw source
SELECT
    user_id,
    user_name,
    signup_date
FROM smelt.source('raw.users')
