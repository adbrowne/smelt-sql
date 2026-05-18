-- Per-day distinct-user and identified-event counts under each of the three
-- identity-resolution algorithms surfaced in gold/eventstream_with_identity.
-- One row per event_date. The identified_events_* columns count events whose
-- corresponding identity column resolved to a non-null user; the dau_*
-- columns count distinct non-null users.
--
-- Invariants the upstream algorithms guarantee:
--   identified_events_forward_only
--     ≤ identified_events_backward_fill
--     ≤ identified_events_connected_components
-- on every day, by subsumption: every event that forward-only identifies is
-- also identified by backward-fill (backward-fill considers the same signed-in
-- observation across all sessions on the device, never fewer); every event
-- that backward-fill identifies is also identified by connected-components
-- (connected-components clusters across devices; a device with a backward-fill
-- canonical user is a non-empty graph node, so it has a cluster).
--
-- DAU is NOT monotonic in the same direction. Connected-components clusters
-- distinct users together (the cluster representative is the smallest
-- user_id), so dau_connected_components can be ≤ dau_backward_fill when a
-- cluster spans distinct users. The mart surfaces both counts so the
-- algorithmic tradeoff is observable: identified_events measures reach
-- (monotonic), dau measures cardinality after cluster collapse
-- (non-monotonic in the cross-device-cluster case).
SELECT
    event_date,
    COUNT(*) AS total_events,
    COUNT(DISTINCT forward_only_user_id) AS dau_forward_only,
    COUNT(DISTINCT backward_fill_user_id) AS dau_backward_fill,
    COUNT(DISTINCT connected_components_user_id) AS dau_connected_components,
    COUNT(*) FILTER (WHERE forward_only_user_id IS NOT NULL) AS identified_events_forward_only,
    COUNT(*) FILTER (WHERE backward_fill_user_id IS NOT NULL) AS identified_events_backward_fill,
    COUNT(*) FILTER (WHERE connected_components_user_id IS NOT NULL) AS identified_events_connected_components
FROM smelt.gold.eventstream_with_identity
GROUP BY event_date
ORDER BY event_date
