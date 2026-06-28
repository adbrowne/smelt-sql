-- @materialization: view
-- Demonstrates a pipe query in CTE position: the CTE body is a FROM-first
-- pipe query, exercising the spec §"Where a pipe query may appear" — CTE body
-- placement.  The outer query is standard SQL that reads from the CTE.
WITH clicks AS (
    FROM smelt.raw_events
    |> WHERE event_type = 'click'
    |> SELECT user_id, event_time
)
SELECT user_id, COUNT(*) AS click_count
FROM clicks
GROUP BY user_id
