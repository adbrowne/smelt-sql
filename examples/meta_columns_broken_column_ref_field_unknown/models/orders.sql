-- Upstream model: provides a resolvable schema for smelt.columns_of.
-- Columns: id (INTEGER), customer_name (VARCHAR), amount (DOUBLE).
SELECT
    id,
    customer_name,
    amount
FROM smelt.sources.raw.orders
