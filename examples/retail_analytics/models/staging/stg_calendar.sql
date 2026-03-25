-- Calendar dimension (sample dates via UNION ALL; parser lacks generate_series support)
WITH date_spine AS (
    SELECT CAST('2024-07-01' AS DATE) AS calendar_date
    UNION ALL SELECT CAST('2024-07-02' AS DATE)
    UNION ALL SELECT CAST('2024-07-03' AS DATE)
    UNION ALL SELECT CAST('2024-07-15' AS DATE)
    UNION ALL SELECT CAST('2024-08-01' AS DATE)
    UNION ALL SELECT CAST('2024-08-15' AS DATE)
    UNION ALL SELECT CAST('2024-09-01' AS DATE)
    UNION ALL SELECT CAST('2024-09-15' AS DATE)
    UNION ALL SELECT CAST('2024-09-28' AS DATE)
)

SELECT
    calendar_date,
    YEAR(calendar_date) AS year,
    MONTH(calendar_date) AS month,
    DAY(calendar_date) AS day_of_month,
    DAYOFWEEK(calendar_date) AS day_of_week,
    QUARTER(calendar_date) AS quarter
FROM date_spine
