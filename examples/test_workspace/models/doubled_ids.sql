-- Phase 5 (nullability-soundness): calls double_id with a NOT NULL source column.
-- event_id is declared nullable: false in the source; the NOT NULL parameter
-- requirement is satisfied — should produce no diagnostics.
SELECT smelt.functions.double_id(event_id) AS doubled_event_id
FROM smelt.raw_events
