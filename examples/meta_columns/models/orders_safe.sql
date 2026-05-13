-- Uses coalesce_numeric to ensure all numeric columns have NULL replaced with 0.
-- The function is called via smelt.functions.coalesce_numeric and spread into
-- the SELECT list with the `...` operator.
--
-- smelt.orders refers to models/orders.sql (path form without subdirectory).
-- At expansion time smelt.columns_of resolves the orders model column list,
-- filter keeps only numeric columns (id, amount, discount), and map generates
-- COALESCE(col, 0) AS col for each.
SELECT
    customer_name,
    ...smelt.functions.coalesce_numeric(smelt.orders)
FROM smelt.orders
