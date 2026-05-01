-- Staging: enrich raw orders with the human-readable status name.
--
-- This model is materialization: table (configured in smelt.yml). It joins a
-- `smelt.source(...)` with a `smelt.ref(...)` that points at a seed
-- (order_statuses), so the CLI dependency validator must accept seeds as
-- valid ref targets (bug #2). Re-running `smelt build` must succeed without
-- deleting the .duckdb file (bug #1).
SELECT
    o.order_id,
    o.customer_id,
    o.status_code,
    s.status_name,
    o.gross_amount,
    o.qty,
    -- Derived column: NOT in sources.yml. Downstream models reference this
    -- through `smelt.ref('stg_orders').line_revenue`, exercising the path
    -- where TypeContext must learn column types from upstream MODELS, not
    -- just from sources.yml. (Bug #3 sub-case: B8.)
    o.gross_amount * o.qty AS line_revenue
FROM smelt.sources.raw.orders AS o
LEFT JOIN smelt.models.order_statuses AS s ON o.status_code = s.status_code

