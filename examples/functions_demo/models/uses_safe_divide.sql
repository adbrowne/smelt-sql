-- Phase 6 fixture: exercise the `smelt.fn.*` call surface end-to-end.
-- Both `user_id` and `event_id` are INTEGER columns from `source.events`
-- (see ../sources.yml), which widen into `Expr<Numeric>` — the declared
-- parameter constraint on `safe_divide`. No diagnostic should fire.
--
-- Phase 7 addendum: also exercise the seed built-ins in the canonical
-- registry (LOWER, UPPER, LENGTH on the VARCHAR `event_type`; ABS on the
-- INTEGER `event_id`). These calls still flow through the hand-written
-- inference match; Phase 9 rewires them through the new registry.
SELECT
    smelt.fn.safe_divide(user_id, event_id) AS safe_ratio,
    LOWER(event_type) AS event_type_lower,
    UPPER(event_type) AS event_type_upper,
    LENGTH(event_type) AS event_type_len,
    ABS(event_id) AS event_id_abs
FROM smelt.source('source.events')
