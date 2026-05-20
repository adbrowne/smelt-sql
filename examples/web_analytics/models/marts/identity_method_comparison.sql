-- Pairwise event-level comparison of the three identity-resolution refinements.
-- One row per pair of methods, reporting the breakdown of every event's
-- (left_amplitude_id, right_amplitude_id) pairing into five disjoint buckets.
-- Every method's amplitude_id is non-null (devices that never signed in
-- resolve to 'd:' || device_id via the eventstream COALESCE), so the buckets
-- partition by whether each side resolved to a real user ('u:') or fell back
-- to the device ('d:'):
--
--   agree_user_events     — both 'u:'-prefixed AND equal
--   agree_device_events   — both 'd:'-prefixed (necessarily equal because both
--                           sides see the same event and therefore the same
--                           device_id; neither method promoted it to a real
--                           user). This is the "both fell back" bucket and
--                           corresponds to the old 'both_null_events' under
--                           the pre-amplitude_id schema.
--   disagree_events       — both 'u:'-prefixed AND not equal (each method
--                           picked a different real user)
--   only_left_user        — left 'u:'-prefixed, right 'd:'-prefixed (left
--                           promoted to a real user, right fell back)
--   only_right_user       — left 'd:'-prefixed, right 'u:'-prefixed
--
-- The five buckets sum to total_events. Expected qualitative shape under the
-- three algorithms' resolution rules:
--
--   forward_vs_backward — disagree_events ≥ 0. Forward-only resolves a session
--     to its latest in-session signed-in user; backward-fill elects the
--     per-device most-frequent user. On a device with two distinct signed-in
--     users where the most-frequent user is not the latest in-session signin
--     of a given session, the two algorithms disagree on every event of that
--     session. only_right_user is the dominant non-agree bucket because
--     backward-fill promotes many events to a real user that forward-only
--     leaves as the device fallback.
--
--   forward_vs_connected — disagree_events ≥ 0. Forward-only resolves to the
--     latest in-session signed-in user; connected-components resolves to the
--     cluster representative (smallest user_id in the cluster). These differ
--     whenever the device is in a multi-user cluster whose minimum is not the
--     latest in-session signed-in user.
--
--   backward_vs_connected — only_left_user = only_right_user = 0 (every
--     device with a backward-fill canonical user is in the connected-components
--     graph too — both algorithms read the same silver/device_user_edges, so
--     they promote the same set of events from 'd:' to 'u:'). disagree_events
--     captures the cluster-collapse: backward-fill's per-device canonical
--     user differs from the cluster representative whenever the cluster spans
--     devices and the device's local election is not the cluster-minimum user.
SELECT
    'forward_vs_backward' AS comparison_name,
    COUNT(*) AS total_events,
    COUNT(*) FILTER (WHERE forward_only_amplitude_id LIKE 'u:%' AND backward_fill_amplitude_id LIKE 'u:%' AND forward_only_amplitude_id = backward_fill_amplitude_id) AS agree_user_events,
    COUNT(*) FILTER (WHERE forward_only_amplitude_id LIKE 'd:%' AND backward_fill_amplitude_id LIKE 'd:%') AS agree_device_events,
    COUNT(*) FILTER (WHERE forward_only_amplitude_id LIKE 'u:%' AND backward_fill_amplitude_id LIKE 'u:%' AND forward_only_amplitude_id != backward_fill_amplitude_id) AS disagree_events,
    COUNT(*) FILTER (WHERE forward_only_amplitude_id LIKE 'u:%' AND backward_fill_amplitude_id LIKE 'd:%') AS only_left_user,
    COUNT(*) FILTER (WHERE forward_only_amplitude_id LIKE 'd:%' AND backward_fill_amplitude_id LIKE 'u:%') AS only_right_user
FROM smelt.gold.eventstream_with_identity

UNION ALL

SELECT
    'forward_vs_connected',
    COUNT(*),
    COUNT(*) FILTER (WHERE forward_only_amplitude_id LIKE 'u:%' AND connected_components_amplitude_id LIKE 'u:%' AND forward_only_amplitude_id = connected_components_amplitude_id),
    COUNT(*) FILTER (WHERE forward_only_amplitude_id LIKE 'd:%' AND connected_components_amplitude_id LIKE 'd:%'),
    COUNT(*) FILTER (WHERE forward_only_amplitude_id LIKE 'u:%' AND connected_components_amplitude_id LIKE 'u:%' AND forward_only_amplitude_id != connected_components_amplitude_id),
    COUNT(*) FILTER (WHERE forward_only_amplitude_id LIKE 'u:%' AND connected_components_amplitude_id LIKE 'd:%'),
    COUNT(*) FILTER (WHERE forward_only_amplitude_id LIKE 'd:%' AND connected_components_amplitude_id LIKE 'u:%')
FROM smelt.gold.eventstream_with_identity

UNION ALL

SELECT
    'backward_vs_connected',
    COUNT(*),
    COUNT(*) FILTER (WHERE backward_fill_amplitude_id LIKE 'u:%' AND connected_components_amplitude_id LIKE 'u:%' AND backward_fill_amplitude_id = connected_components_amplitude_id),
    COUNT(*) FILTER (WHERE backward_fill_amplitude_id LIKE 'd:%' AND connected_components_amplitude_id LIKE 'd:%'),
    COUNT(*) FILTER (WHERE backward_fill_amplitude_id LIKE 'u:%' AND connected_components_amplitude_id LIKE 'u:%' AND backward_fill_amplitude_id != connected_components_amplitude_id),
    COUNT(*) FILTER (WHERE backward_fill_amplitude_id LIKE 'u:%' AND connected_components_amplitude_id LIKE 'd:%'),
    COUNT(*) FILTER (WHERE backward_fill_amplitude_id LIKE 'd:%' AND connected_components_amplitude_id LIKE 'u:%')
FROM smelt.gold.eventstream_with_identity
