-- Per-device connected-components identity resolution via bipartite-graph
-- union-find. From silver/device_user_edges (the (device, user) co-occurrence
-- evidence over all signed-in events), build the bipartite graph where each
-- edge connects one device node to one user node. Two devices are in the same
-- component if a path of edges connects them through one or more shared users.
-- The cluster representative is 'u:' || the smallest user_id in the component,
-- which doubles as the cluster_id under this v1 convention.
--
-- This is the Amplitude-full identity model — it propagates identity across
-- devices via user co-occurrence. Subsumes backward-fill on every device:
-- a device's backward-fill canonical user appears in its connected component
-- (because the backward-fill election only considers users who signed in on
-- the device itself, and the connected component includes all such users by
-- definition).
--
-- Devices that never had a signed-in event do not appear in
-- silver/device_user_edges and therefore not in this table; the eventstream
-- layer COALESCEs the NULL LEFT-JOIN result into the device-prefix amplitude_id.
--
-- The propagation is implemented as an iter-unrolled label-propagation over
-- 8 explicit passes (iter0 through iter8). Each pass recomputes every device's
-- label as the MIN over its own current label and the labels of all devices
-- that share any user with it. Eight passes provide a 256-fold expansion of
-- cluster diameter, far above the synthetic dataset's expected graph diameter
-- (3-5 hops at the 60/25/10/5 co-occurrence weights). DuckDB's recursive-CTE
-- engine does not permit aggregates (MIN, GROUP BY) in the recursive term, so
-- the iter-unrolled form is used rather than a WITH RECURSIVE CTE. A future
-- revision may replace this with a true fixed-point form if DuckDB lifts the
-- aggregate restriction or a DuckDB-compatible recursive aggregate pattern
-- becomes available.
--
-- Output columns:
--   device_id                       - the device
--   connected_components_amplitude_id  - 'u:' || smallest user_id in the device's cluster
--   connected_components_cluster_id    - cluster label (= amplitude_id above in v1)
--
-- Both identity columns are equal in v1 (both are 'u:' || MIN user_id in
-- cluster) and are surfaced separately so a future probabilistic-stitching
-- alternative can decouple them without reshuffling the eventstream schema.
WITH edges AS (
    SELECT device_id, user_id
    FROM smelt.silver.device_user_edges
),
-- Seed: each device's initial label is the MIN user_id seen on it.
iter0 AS (
    SELECT device_id, MIN(user_id) AS label
    FROM edges
    GROUP BY device_id
),
-- Each subsequent iter propagates: a device's label becomes the MIN over its
-- own current label and the labels of every device that shares any user with it.
iter1 AS (
    SELECT e.device_id,
           MIN(CASE WHEN i.label < i2.label THEN i.label ELSE i2.label END) AS label
    FROM iter0 i
    JOIN edges e ON e.device_id = i.device_id
    JOIN edges e2 ON e2.user_id = e.user_id
    JOIN iter0 i2 ON i2.device_id = e2.device_id
    GROUP BY e.device_id
),
iter2 AS (
    SELECT e.device_id,
           MIN(CASE WHEN i.label < i2.label THEN i.label ELSE i2.label END) AS label
    FROM iter1 i
    JOIN edges e ON e.device_id = i.device_id
    JOIN edges e2 ON e2.user_id = e.user_id
    JOIN iter1 i2 ON i2.device_id = e2.device_id
    GROUP BY e.device_id
),
iter3 AS (
    SELECT e.device_id,
           MIN(CASE WHEN i.label < i2.label THEN i.label ELSE i2.label END) AS label
    FROM iter2 i
    JOIN edges e ON e.device_id = i.device_id
    JOIN edges e2 ON e2.user_id = e.user_id
    JOIN iter2 i2 ON i2.device_id = e2.device_id
    GROUP BY e.device_id
),
iter4 AS (
    SELECT e.device_id,
           MIN(CASE WHEN i.label < i2.label THEN i.label ELSE i2.label END) AS label
    FROM iter3 i
    JOIN edges e ON e.device_id = i.device_id
    JOIN edges e2 ON e2.user_id = e.user_id
    JOIN iter3 i2 ON i2.device_id = e2.device_id
    GROUP BY e.device_id
),
iter5 AS (
    SELECT e.device_id,
           MIN(CASE WHEN i.label < i2.label THEN i.label ELSE i2.label END) AS label
    FROM iter4 i
    JOIN edges e ON e.device_id = i.device_id
    JOIN edges e2 ON e2.user_id = e.user_id
    JOIN iter4 i2 ON i2.device_id = e2.device_id
    GROUP BY e.device_id
),
iter6 AS (
    SELECT e.device_id,
           MIN(CASE WHEN i.label < i2.label THEN i.label ELSE i2.label END) AS label
    FROM iter5 i
    JOIN edges e ON e.device_id = i.device_id
    JOIN edges e2 ON e2.user_id = e.user_id
    JOIN iter5 i2 ON i2.device_id = e2.device_id
    GROUP BY e.device_id
),
iter7 AS (
    SELECT e.device_id,
           MIN(CASE WHEN i.label < i2.label THEN i.label ELSE i2.label END) AS label
    FROM iter6 i
    JOIN edges e ON e.device_id = i.device_id
    JOIN edges e2 ON e2.user_id = e.user_id
    JOIN iter6 i2 ON i2.device_id = e2.device_id
    GROUP BY e.device_id
),
iter8 AS (
    SELECT e.device_id,
           MIN(CASE WHEN i.label < i2.label THEN i.label ELSE i2.label END) AS label
    FROM iter7 i
    JOIN edges e ON e.device_id = i.device_id
    JOIN edges e2 ON e2.user_id = e.user_id
    JOIN iter7 i2 ON i2.device_id = e2.device_id
    GROUP BY e.device_id
)
SELECT
    device_id,
    'u:' || CAST(label AS VARCHAR) AS connected_components_amplitude_id,
    'u:' || CAST(label AS VARCHAR) AS connected_components_cluster_id
FROM iter8
