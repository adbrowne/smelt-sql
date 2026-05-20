-- Per-day distinct-amplitude_id and identified-event counts under the
-- no-merging baseline (raw) and the three refinement algorithms surfaced in
-- gold/eventstream_with_identity. One row per event_date. Every event in the
-- eventstream has a non-null amplitude_id under every method (devices that
-- never signed in resolve to 'd:' || device_id via the eventstream COALESCE),
-- so the columns measure a fully-populated identity space rather than
-- "events the method bothered to tag".
--
-- Invariants the upstream pipeline guarantees:
--
--   identified_events_raw
--     ≤ identified_events_forward_only
--     ≤ identified_events_backward_fill
--     = identified_events_connected_components
-- on every day. identified_events_* counts events whose method-amplitude_id
-- is 'u:'-prefixed (i.e., resolved to a real user, not the device fallback).
-- Each method only ever adds 'u:'-prefixed identifications: forward_only
-- promotes anonymous events inside a signed-in session, backward_fill promotes
-- anonymous events on a device that has ever had a signed-in user, and
-- connected_components shares the same set of 'u:'-resolved events as
-- backward_fill (both consume the same silver/device_user_edges).
--
--   dau_backward_fill ≥ dau_connected_components
-- on every day. connected_components is a strict per-device coarsening of
-- backward_fill (each device gets its cluster's representative instead of
-- its own canonical user), so any pair of devices in the same cluster
-- collapses to a single id under connected_components.
--
-- DAU is otherwise NOT monotonic across the four methods per day:
--   - raw vs forward_only: forward_only "inherits" identities across day
--     boundaries (an anon event on Tuesday in a session that signed-in on
--     Monday resolves to 'u:U'), which raw never does. On days dominated by
--     cross-day session continuations, forward_only can have strictly more
--     distinct ids than raw.
--   - forward_only vs backward_fill: on a day where a device has only
--     anon-only sessions, forward_only resolves those events to 'd:device_id'
--     while backward_fill resolves them to the device's canonical user.
--     Backward_fill can "recover" 'u:'-namespace identities that
--     forward_only loses, so dau_backward_fill can exceed dau_forward_only.
--
-- Both shapes are intentional and dataset-dependent. The four methods are
-- different partitions of the same identity space, not a single chain of
-- successive refinements. The README narrative explains the cost/coverage
-- tradeoffs.
SELECT
    event_date,
    COUNT(*) AS total_events,
    COUNT(DISTINCT amplitude_id) AS dau_raw,
    COUNT(DISTINCT forward_only_amplitude_id) AS dau_forward_only,
    COUNT(DISTINCT backward_fill_amplitude_id) AS dau_backward_fill,
    COUNT(DISTINCT connected_components_amplitude_id) AS dau_connected_components,
    COUNT(*) FILTER (WHERE amplitude_id LIKE 'u:%') AS identified_events_raw,
    COUNT(*) FILTER (WHERE forward_only_amplitude_id LIKE 'u:%') AS identified_events_forward_only,
    COUNT(*) FILTER (WHERE backward_fill_amplitude_id LIKE 'u:%') AS identified_events_backward_fill,
    COUNT(*) FILTER (WHERE connected_components_amplitude_id LIKE 'u:%') AS identified_events_connected_components
FROM smelt.gold.eventstream_with_identity
GROUP BY event_date
ORDER BY event_date
