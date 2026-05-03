-- Mart: revenue aggregates over the staging table.
--
-- The natural SQL below should yield:
--   net_revenue   DOUBLE  (SUM over a DOUBLE column stays DOUBLE)
--   unique_orders BIGINT  (COUNT(DISTINCT ...) is always BIGINT)
--   gross_value   DOUBLE  (SUM of DOUBLE * INTEGER stays DOUBLE)
--
-- On main the `_smelt_typed` wrapper narrows these: net_revenue becomes
-- BIGINT, which silently truncates the fractional cents (bug #3).
-- DO NOT add manual CAST(... AS DOUBLE) workarounds here — that is the bug
-- this regression test is designed to catch.
SELECT
    SUM(gross_amount) AS net_revenue,
    COUNT(DISTINCT order_id) AS unique_orders,
    SUM(gross_amount * qty) AS gross_value
FROM smelt.stg_orders

