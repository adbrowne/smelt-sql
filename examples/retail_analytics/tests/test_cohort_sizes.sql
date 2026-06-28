-- Cohort sizes test: cohort_base + cohort_sizes CTE chain.
-- MIN(order_date) per customer gives the cohort date; COUNT(DISTINCT customer_id)
-- gives the cohort size.
--
-- Exercises the real mart_cohort_retention model's internal cohort_sizes CTE via
-- the test-local # operator. The CTE's upstream chain (cohort_base) runs as
-- written, computing MIN(order_date) per customer from the mocked rows; PASSING
-- mocks the model's external dep (intermediate.int_order_enriched), keyed by its
-- real address path.

smelt.test test_cohort_sizes AS (
    SELECT cohort_date, cohort_size
    FROM smelt.marts.mart_cohort_retention#cohort_sizes
)
PASSING intermediate.int_order_enriched AS (
    {customer_id: 1, order_date: '2024-01-01'},
    {customer_id: 2, order_date: '2024-01-01'},
    {customer_id: 3, order_date: '2024-02-01'}
)
EXPECT (
    {cohort_date: '2024-01-01', cohort_size: 2},
    {cohort_date: '2024-02-01', cohort_size: 1}
)
