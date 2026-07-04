---
materialization: table
timeseries:
  event_time_column: order_ts
  partition_column: order_date
  granularity: day
refresh: batched
---

-- D1 safety bypass: outer SQL has no OVER clause, so the safety check passes.
-- But the expanded SQL would have SUM(...) OVER (PARTITION BY user_id ...) which
-- is NOT partition-aligned to order_date → should be rejected by spec §"Safety checks".
-- Function call path: smelt.functions.<fn_name> (directory only, no file stem).
SELECT
    order_date,
    user_id,
    order_ts,
    smelt.functions.user_running_total(amount, user_id) AS running_total
FROM smelt.sources.orders
