---
materialization: table
timeseries:
  event_time_column: partition_key
  partition_column: partition_key
  granularity: day
  assert_monotonic: true
refresh: incremental
grain: partition
---

-- `ABS(...)` is a real, recognized SQL function (so type inference is
-- unaffected), but it is opaque to the static monotonicity classifier — no
-- rule in `smelt-logical::analysis::monotonicity` recognizes it, so without
-- `assert_monotonic: true` the join driving-fact resolution would refuse
-- this model outright. The declaration asserts that the projected
-- expression is still monotone non-decreasing in `f.partition_key`,
-- admitting the join pushdown.
SELECT
    ABS(f.partition_key) AS partition_key,
    f.user_id,
    g.attribute
FROM smelt.sources.fact f
JOIN smelt.sources.lookup g ON f.user_id = g.user_id
