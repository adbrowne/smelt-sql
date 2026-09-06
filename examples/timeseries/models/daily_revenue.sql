-- Daily revenue aggregation by user
-- Demonstrates incremental materialization with daily partitions
--
-- This model aggregates transaction revenue by date and user.
-- With incremental materialization enabled, only new/updated partitions
-- are reprocessed, dramatically reducing compute cost.
--
-- Joins the users dimension so dashboards can label a row without a second
-- join. Valid SQL, no diagnostic fires — the cost shows up only in the
-- derived properties, which is what the property diff is for.

SELECT
    CAST(t.transaction_timestamp AS DATE) as revenue_date,
    t.user_id,
    u.user_name,
    COUNT(*) as transaction_count,
    SUM(t.amount) as total_revenue,
    AVG(t.amount) as avg_transaction_amount,
    MIN(t.transaction_timestamp) as first_transaction,
    MAX(t.transaction_timestamp) as last_transaction,
FROM smelt.sources.raw.transactions AS t
JOIN smelt.sources.raw.users AS u ON t.user_id = u.user_id
WHERE t.transaction_timestamp IS NOT NULL
GROUP BY 1, 2, 3
ORDER BY 1, 2

