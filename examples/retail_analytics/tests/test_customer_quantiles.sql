--- name: test_customer_quantiles ---
materialization: test
test:
  model: int_customer_segments
  target_cte: customer_quantiles
  inputs:
    customer_metrics:
      - {customer_id: 1, customer_segment: Premium, order_count: 10, total_revenue: 1000.0, total_net_revenue: 900.0}
      - {customer_id: 2, customer_segment: Standard, order_count: 5, total_revenue: 500.0, total_net_revenue: 450.0}
      - {customer_id: 3, customer_segment: Basic, order_count: 2, total_revenue: 100.0, total_net_revenue: 90.0}
      - {customer_id: 4, customer_segment: Premium, order_count: 8, total_revenue: 800.0, total_net_revenue: 720.0}
  expect:
    - {customer_id: 1, revenue_decile: 1, frequency_decile: 1}
    - {customer_id: 4, revenue_decile: 2, frequency_decile: 2}
    - {customer_id: 2, revenue_decile: 3, frequency_decile: 3}
    - {customer_id: 3, revenue_decile: 4, frequency_decile: 4}
---
