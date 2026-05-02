-- Transactions source model
-- Maps raw transactions to the smelt namespace

SELECT
    transaction_id,
    user_id,
    amount,
    transaction_timestamp,
    transaction_type
FROM smelt.sources.raw.transactions

