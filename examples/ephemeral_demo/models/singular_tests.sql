--- name: assert_positive_revenue ---
materialization: test
---
-- Singular test: verify no negative revenue values exist.
-- Returns failing rows — 0 rows = pass.
SELECT order_date, total_revenue
FROM main.daily_revenue
WHERE total_revenue < 0

