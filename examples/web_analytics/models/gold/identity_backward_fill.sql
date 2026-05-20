-- Per-device canonical-user election. From silver/device_user_edges (the
-- (device, user) co-occurrence evidence over all signed-in events), pick the
-- user_id with the highest event_count for each device; ties broken by
-- earliest first_seen, then by smallest user_id. The chosen user is the
-- device's canonical user under the Amplitude-basic backward-fill model —
-- once a user has signed in on a device, every event on that device
-- retroactively belongs to that user (regardless of session or whether the
-- event itself was signed-in).
--
-- The final user_id tiebreaker ensures deterministic output across rebuilds
-- in the (rare) case that two users share both event_count and first_seen
-- on the same device. Without it, DISTINCT ON's choice would depend on
-- storage order.
--
-- backward_fill_amplitude_id is 'u:' || the elected user_id. Devices that
-- never had a signed-in event do not appear in silver/device_user_edges, and
-- therefore do not appear in this table either. The eventstream layer
-- COALESCEs the NULL LEFT-JOIN result into the device-prefix amplitude_id.
SELECT DISTINCT ON (device_id)
    device_id,
    'u:' || CAST(user_id AS VARCHAR) AS backward_fill_amplitude_id
FROM smelt.silver.device_user_edges_cumulative
ORDER BY device_id, event_count DESC, first_seen ASC, user_id ASC
