---
name: orders_summary
materialization: table
---
SELECT
  DATE(order_date) AS order_day,
  COUNT(*) AS order_count,
  SUM(amount) AS total_amount
FROM smelt.raw_orders
GROUP BY 1
