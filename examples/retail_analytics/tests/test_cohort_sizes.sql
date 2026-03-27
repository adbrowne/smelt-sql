--- name: test_cohort_sizes ---
materialization: test
test:
  model: mart_cohort_retention
  target_cte: cohort_sizes
  inputs:
    cohort_base:
      - {customer_id: 1, cohort_date: '2024-01-01'}
      - {customer_id: 2, cohort_date: '2024-01-01'}
      - {customer_id: 3, cohort_date: '2024-02-01'}
  expect:
    - {cohort_date: '2024-01-01', cohort_size: 2}
    - {cohort_date: '2024-02-01', cohort_size: 1}
---
