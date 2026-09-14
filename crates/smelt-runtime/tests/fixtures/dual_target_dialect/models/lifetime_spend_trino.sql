---
materialization: table
refresh: incremental
grain: key
---
SELECT user_id, SUM(amount) AS lifetime_spend
FROM smelt.sources.payments
GROUP BY user_id
