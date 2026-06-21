--- name: test_connected_components_resolution_invariants ---
materialization: test
test:
  model: identity_connected_components
  inputs:
    silver.device_user_edges:
      # Cluster 1: single device, single user (degenerate base case)
      - {device_id: 1, user_id: 100, event_count: 1, first_seen: '2026-04-01 09:00:00', last_seen: '2026-04-01 09:01:00'}

      # Cluster 2: single device, two users → joined through device 2
      # Cluster = {200, 201}; representative = MIN(200, 201) = 200 → 'u:200'
      - {device_id: 2, user_id: 200, event_count: 1, first_seen: '2026-04-01 10:00:00', last_seen: '2026-04-01 10:01:00'}
      - {device_id: 2, user_id: 201, event_count: 1, first_seen: '2026-04-01 10:10:00', last_seen: '2026-04-01 10:11:00'}

      # Cluster 3: two devices joined through user 301
      # Cluster = {300, 301}; both device 3 and device 4 resolve to 'u:300'
      - {device_id: 3, user_id: 300, event_count: 1, first_seen: '2026-04-01 11:00:00', last_seen: '2026-04-01 11:01:00'}
      - {device_id: 3, user_id: 301, event_count: 1, first_seen: '2026-04-01 11:10:00', last_seen: '2026-04-01 11:11:00'}
      - {device_id: 4, user_id: 301, event_count: 1, first_seen: '2026-04-01 11:20:00', last_seen: '2026-04-01 11:21:00'}

      # Cluster 4: three-device chain (transitive closure)
      # Device 5 ↔ user 501 ↔ Device 6 ↔ user 502 ↔ Device 7
      # Cluster = {500, 501, 502, 503}; representative = 'u:500'.
      # All three devices resolve to 'u:500'. This case forces propagation
      # to converge over ≥ 2 iterations; a single-iter implementation would
      # fail because device 7 cannot reach device 5 in one hop.
      - {device_id: 5, user_id: 500, event_count: 1, first_seen: '2026-04-01 12:00:00', last_seen: '2026-04-01 12:01:00'}
      - {device_id: 5, user_id: 501, event_count: 1, first_seen: '2026-04-01 12:10:00', last_seen: '2026-04-01 12:11:00'}
      - {device_id: 6, user_id: 501, event_count: 1, first_seen: '2026-04-01 12:20:00', last_seen: '2026-04-01 12:21:00'}
      - {device_id: 6, user_id: 502, event_count: 1, first_seen: '2026-04-01 12:30:00', last_seen: '2026-04-01 12:31:00'}
      - {device_id: 7, user_id: 502, event_count: 1, first_seen: '2026-04-01 12:40:00', last_seen: '2026-04-01 12:41:00'}
      - {device_id: 7, user_id: 503, event_count: 1, first_seen: '2026-04-01 12:50:00', last_seen: '2026-04-01 12:51:00'}

      # Cluster 5: isolated user retains identity (negative test — no spurious merging)
      - {device_id: 8, user_id: 600, event_count: 1, first_seen: '2026-04-01 13:00:00', last_seen: '2026-04-01 13:01:00'}
  expect:
    - {device_id: 1, connected_components_amplitude_id: 'u:100', connected_components_cluster_id: 'u:100'}  # degenerate
    - {device_id: 2, connected_components_amplitude_id: 'u:200', connected_components_cluster_id: 'u:200'}  # MIN(200, 201)
    - {device_id: 3, connected_components_amplitude_id: 'u:300', connected_components_cluster_id: 'u:300'}  # MIN(300, 301)
    - {device_id: 4, connected_components_amplitude_id: 'u:300', connected_components_cluster_id: 'u:300'}  # via user 301 ↔ device 3
    - {device_id: 5, connected_components_amplitude_id: 'u:500', connected_components_cluster_id: 'u:500'}  # chain head
    - {device_id: 6, connected_components_amplitude_id: 'u:500', connected_components_cluster_id: 'u:500'}  # transitive
    - {device_id: 7, connected_components_amplitude_id: 'u:500', connected_components_cluster_id: 'u:500'}  # 2-hop transitive
    - {device_id: 8, connected_components_amplitude_id: 'u:600', connected_components_cluster_id: 'u:600'}  # isolated
---
