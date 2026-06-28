-- Connected-components identity resolution invariants.
-- Exercises the real gold/identity_connected_components model: the assertion
-- query selects from smelt.gold.identity_connected_components and PASSING mocks
-- its single external dep (silver.device_user_edges). The model's 8-iteration
-- label-propagation runs as written; the cluster representative is
-- 'u:' || MIN(user_id) in the connected component.

smelt.test test_connected_components_resolution_invariants AS (
    SELECT device_id, connected_components_amplitude_id, connected_components_cluster_id
    FROM smelt.gold.identity_connected_components
)
PASSING silver.device_user_edges AS (
    -- Cluster 1: single device, single user (degenerate base case)
    {device_id: 1, user_id: 100},
    -- Cluster 2: single device, two users → MIN(200, 201) = 200
    {device_id: 2, user_id: 200},
    {device_id: 2, user_id: 201},
    -- Cluster 3: two devices joined through user 301 → MIN(300, 301) = 300
    {device_id: 3, user_id: 300},
    {device_id: 3, user_id: 301},
    {device_id: 4, user_id: 301},
    -- Cluster 4: three-device chain (transitive closure); representative 'u:500'.
    -- Device 5 ↔ user 501 ↔ Device 6 ↔ user 502 ↔ Device 7 — forces propagation
    -- to converge over ≥ 2 iterations.
    {device_id: 5, user_id: 500},
    {device_id: 5, user_id: 501},
    {device_id: 6, user_id: 501},
    {device_id: 6, user_id: 502},
    {device_id: 7, user_id: 502},
    {device_id: 7, user_id: 503},
    -- Cluster 5: isolated user retains identity (no spurious merging)
    {device_id: 8, user_id: 600}
)
EXPECT (
    {device_id: 1, connected_components_amplitude_id: 'u:100', connected_components_cluster_id: 'u:100'},  -- degenerate
    {device_id: 2, connected_components_amplitude_id: 'u:200', connected_components_cluster_id: 'u:200'},  -- MIN(200, 201)
    {device_id: 3, connected_components_amplitude_id: 'u:300', connected_components_cluster_id: 'u:300'},  -- MIN(300, 301)
    {device_id: 4, connected_components_amplitude_id: 'u:300', connected_components_cluster_id: 'u:300'},  -- via user 301 ↔ device 3
    {device_id: 5, connected_components_amplitude_id: 'u:500', connected_components_cluster_id: 'u:500'},  -- chain head
    {device_id: 6, connected_components_amplitude_id: 'u:500', connected_components_cluster_id: 'u:500'},  -- transitive
    {device_id: 7, connected_components_amplitude_id: 'u:500', connected_components_cluster_id: 'u:500'},  -- 2-hop transitive
    {device_id: 8, connected_components_amplitude_id: 'u:600', connected_components_cluster_id: 'u:600'}   -- isolated
)
