-- Staged customer dimension with type casting and cleanup
SELECT
    customer_id,
    COALESCE(name_prefix, '') AS name_prefix,
    CAST(birth_year AS INTEGER) AS birth_year,
    CASE
        WHEN gender = 'M' THEN 'Male'
        WHEN gender = 'F' THEN 'Female'
        ELSE 'Other'
    END AS gender,
    email_domain,
    country,
    city,
    CAST(signup_date AS DATE) AS signup_date,
    COALESCE(segment, 'Unknown') AS segment
FROM smelt.sources.raw.customers

