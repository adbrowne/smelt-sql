-- Phase 36 fixture: calls event_hour with a source that has ts + user_id.
-- The struct parameter {ts: Timestamp, ..r} is satisfied by source.events
-- which declares ts: TIMESTAMP, user_id: INTEGER, event_id: INTEGER, and
-- event_type: VARCHAR. The row variable `r` captures the extra fields.
SELECT smelt.functions.event_hour(smelt.sources.source.events) AS hour

