-- Phase 6 fixture: exercise the `smelt.fn.*` call surface end-to-end.
-- Both `user_id` and `event_id` are INTEGER columns from `source.events`
-- (see ../sources.yml), which widen into `Expr<Numeric>` — the declared
-- parameter constraint on `safe_divide`. No diagnostic should fire.
SELECT smelt.fn.safe_divide(user_id, event_id) AS safe_ratio
FROM smelt.source('source.events')
