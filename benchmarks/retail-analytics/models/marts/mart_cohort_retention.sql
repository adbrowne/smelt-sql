-- Cohort retention analysis with CTE chain, FIRST_VALUE, and CROSS JOIN
WITH cohort_base AS (
    SELECT
        customer_id,
        MIN(order_date) AS cohort_date
    FROM smelt.ref('int_order_enriched')
    GROUP BY customer_id
),

periods AS (
    SELECT DISTINCT order_date AS period_date
    FROM smelt.ref('int_order_enriched')
),

cohort_activity AS (
    SELECT
        cb.customer_id,
        cb.cohort_date,
        o.order_date AS activity_date,
        CAST(o.order_date - cb.cohort_date AS INTEGER) AS days_since_cohort
    FROM cohort_base AS cb
    INNER JOIN smelt.ref('int_order_enriched') AS o
        ON cb.customer_id = o.customer_id
),

cohort_sizes AS (
    SELECT
        cohort_date,
        COUNT(DISTINCT customer_id) AS cohort_size
    FROM cohort_base
    GROUP BY cohort_date
)

SELECT
    ca.cohort_date,
    cs.cohort_size,
    ca.days_since_cohort,
    COUNT(DISTINCT ca.customer_id) AS active_customers,
    CAST(COUNT(DISTINCT ca.customer_id) AS FLOAT)
        / CAST(cs.cohort_size AS FLOAT) AS retention_rate
FROM cohort_activity AS ca
INNER JOIN cohort_sizes AS cs ON ca.cohort_date = cs.cohort_date
GROUP BY ca.cohort_date, cs.cohort_size, ca.days_since_cohort
