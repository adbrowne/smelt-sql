-- Daily revenue aggregation by user
-- Demonstrates incremental materialization with daily partitions
--
-- This model aggregates transaction revenue by date and user.
-- With incremental materialization enabled, only new/updated partitions
-- are reprocessed, dramatically reducing compute cost.

SELECT
    CAST(transaction_timestamp AS DATE) as revenue_date,
    user_id,
    COUNT(*) as transaction_count,
    SUM(amount) as total_revenue,
    AVG(amount) as avg_transaction_amount,
    MIN(transaction_timestamp) as first_transaction,
    MAX(transaction_timestamp) as last_transaction,
FROM smelt.source('raw.transactions')
WHERE transaction_timestamp IS NOT NULL
GROUP BY 1, 2
ORDER BY 1, 2
