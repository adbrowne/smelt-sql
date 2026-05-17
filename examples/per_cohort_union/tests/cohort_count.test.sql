---
materialization: test
---
-- Acceptance test: the unioned all_cohorts_unioned table has the same row count
-- as the sum of per-cohort filtered counts from orders.
--
-- This exercises Phase B reducers + Phase C reflection + Phase E1 records +
-- Phase E2 multi-model production end-to-end.
SELECT
    (SELECT COUNT(*) FROM smelt.all_cohorts_unioned)
    = (SELECT SUM(cnt) FROM (
        SELECT COUNT(*) AS cnt FROM smelt.orders WHERE region = 'us-west-2' AND revenue >= 100
        UNION ALL
        SELECT COUNT(*) AS cnt FROM smelt.orders WHERE region = 'us-east-1' AND revenue >= 100
        UNION ALL
        SELECT COUNT(*) AS cnt FROM smelt.orders WHERE region = 'eu-west-1' AND revenue >= 50
      ) AS sub) AS passes
