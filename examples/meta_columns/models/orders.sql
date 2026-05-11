-- Upstream model: order data with a mix of numeric and non-numeric columns.
-- - id: INTEGER (numeric)
-- - customer_name: VARCHAR (non-numeric)
-- - amount: DOUBLE (numeric)
-- - discount: DOUBLE (numeric)
SELECT
    id,
    customer_name,
    amount,
    discount
FROM smelt.sources.raw.orders
