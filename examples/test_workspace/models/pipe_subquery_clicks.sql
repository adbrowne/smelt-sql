-- @materialization: view
-- Demonstrates a pipe query in subquery (parenthesised) position, exercising
-- the spec §"Where a pipe query may appear" — subquery body placement.
-- The outer query is standard SQL that wraps the pipe subquery.
SELECT user_id, event_time
FROM (
    FROM smelt.raw_events
    |> WHERE event_type = 'click'
) filtered
