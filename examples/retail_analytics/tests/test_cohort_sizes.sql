-- D-45: inputs key uses the external smelt.<path> dep (intermediate.int_order_enriched),
-- not the internal CTE name (cohort_base). cohort_base runs as-written, computing
-- MIN(order_date) per customer from the mocked int_order_enriched rows.
--- name: test_cohort_sizes ---
materialization: test
test:
  model: mart_cohort_retention
  target_cte: cohort_sizes
  inputs:
    intermediate.int_order_enriched:
      - {customer_id: 1, order_date: '2024-01-01'}
      - {customer_id: 2, order_date: '2024-01-01'}
      - {customer_id: 3, order_date: '2024-02-01'}
  expect:
    - {cohort_date: '2024-01-01', cohort_size: 2}
    - {cohort_date: '2024-02-01', cohort_size: 1}
---
