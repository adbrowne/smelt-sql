-- Pairwise event-level comparison of the three identity-resolution algorithms.
-- One row per pair of methods, reporting the breakdown of every event's
-- (left_user_id, right_user_id) pairing into five disjoint buckets:
--
--   agree_events          — both non-null AND equal
--   disagree_events       — both non-null AND not equal
--   only_left_identified  — left non-null, right null
--   only_right_identified — left null, right non-null
--   both_null_events      — both null
--
-- The five buckets sum to total_events. The expected shape under the three
-- algorithms' resolution rules:
--
--   forward_vs_backward — disagree_events ≥ 0. Forward-only resolves a session
--     to its latest in-session signed-in user; backward-fill elects the
--     per-device most-frequent user (with first_seen and user_id tiebreaks).
--     On a device with two distinct signed-in users where the most-frequent
--     user is not the latest in-session signin of a given session, the two
--     algorithms disagree on every event of that session that forward-only
--     resolves. only_right_identified is the dominant non-agree bucket:
--     backward-fill identifies many events across sessions that lack an
--     in-session signed-in observation, where forward-only stays null.
--
--   forward_vs_connected — disagree_events ≥ 0. Forward-only resolves to the
--     latest in-session signed-in user; connected-components resolves to the
--     cluster representative (smallest user_id in the cluster). These differ
--     whenever the device is in a multi-user cluster whose minimum is not the
--     latest in-session signed-in user.
--
--   backward_vs_connected — disagree_events ≥ 0. Backward-fill's per-device
--     canonical user differs from the cluster representative whenever the
--     cluster spans devices and the device's local election is not the
--     cluster-minimum user. only_right_identified is 0 (every device with a
--     backward-fill canonical user is in the connected-components graph too —
--     both algorithms read the same silver/device_user_edges).
SELECT
    'forward_vs_backward' AS comparison_name,
    COUNT(*) AS total_events,
    COUNT(*) FILTER (WHERE forward_only_user_id IS NOT NULL AND backward_fill_user_id IS NOT NULL AND forward_only_user_id = backward_fill_user_id) AS agree_events,
    COUNT(*) FILTER (WHERE forward_only_user_id IS NOT NULL AND backward_fill_user_id IS NOT NULL AND forward_only_user_id != backward_fill_user_id) AS disagree_events,
    COUNT(*) FILTER (WHERE forward_only_user_id IS NOT NULL AND backward_fill_user_id IS NULL) AS only_left_identified,
    COUNT(*) FILTER (WHERE forward_only_user_id IS NULL AND backward_fill_user_id IS NOT NULL) AS only_right_identified,
    COUNT(*) FILTER (WHERE forward_only_user_id IS NULL AND backward_fill_user_id IS NULL) AS both_null_events
FROM smelt.gold.eventstream_with_identity

UNION ALL

SELECT
    'forward_vs_connected',
    COUNT(*),
    COUNT(*) FILTER (WHERE forward_only_user_id IS NOT NULL AND connected_components_user_id IS NOT NULL AND forward_only_user_id = connected_components_user_id),
    COUNT(*) FILTER (WHERE forward_only_user_id IS NOT NULL AND connected_components_user_id IS NOT NULL AND forward_only_user_id != connected_components_user_id),
    COUNT(*) FILTER (WHERE forward_only_user_id IS NOT NULL AND connected_components_user_id IS NULL),
    COUNT(*) FILTER (WHERE forward_only_user_id IS NULL AND connected_components_user_id IS NOT NULL),
    COUNT(*) FILTER (WHERE forward_only_user_id IS NULL AND connected_components_user_id IS NULL)
FROM smelt.gold.eventstream_with_identity

UNION ALL

SELECT
    'backward_vs_connected',
    COUNT(*),
    COUNT(*) FILTER (WHERE backward_fill_user_id IS NOT NULL AND connected_components_user_id IS NOT NULL AND backward_fill_user_id = connected_components_user_id),
    COUNT(*) FILTER (WHERE backward_fill_user_id IS NOT NULL AND connected_components_user_id IS NOT NULL AND backward_fill_user_id != connected_components_user_id),
    COUNT(*) FILTER (WHERE backward_fill_user_id IS NOT NULL AND connected_components_user_id IS NULL),
    COUNT(*) FILTER (WHERE backward_fill_user_id IS NULL AND connected_components_user_id IS NOT NULL),
    COUNT(*) FILTER (WHERE backward_fill_user_id IS NULL AND connected_components_user_id IS NULL)
FROM smelt.gold.eventstream_with_identity
