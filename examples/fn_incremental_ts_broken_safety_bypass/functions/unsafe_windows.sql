-- D1 safety-bypass probe: a non-partition-aligned window function inside a function body.
-- The model's partition_column is order_date. This window's PARTITION BY is user_id only
-- (does NOT include order_date) → non-partition-aligned → should be rejected for
-- incremental models per spec §"Safety checks".
-- But the safety check in the run pipeline runs on unexpanded SQL where this OVER is
-- invisible → silent accept. This is a known limitation (BUG-062, needs-review).
smelt.define user_running_total(
    amount: Expr<Numeric>,
    user_id: Expr<Integer>
) -> Expr<Numeric> AS (
    SUM(amount) OVER (PARTITION BY user_id ORDER BY amount)
)
