-- Cleaned user data from raw source
SELECT
    user_id,
    LOWER(email) AS email,
    MD5(email) AS email_hash,
    signup_date,
    plan_type
FROM smelt.sources.raw.users

