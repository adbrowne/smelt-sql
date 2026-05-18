--- name: test_dau_monotonicity_invariants ---
materialization: test
test:
  model: daily_active_users_by_method
  inputs:
    gold_eventstream_with_identity:
      # Day 1 (2026-04-01): 4 events on 2 devices. Only device 1 has a
      # signed-in event (user 100). Forward-only resolves only the in-session
      # signed-in event itself (event 2); backward-fill and connected-
      # components retroactively tag the device's other events on the day.
      # Device 2 stays anonymous everywhere. Day 1 has no cross-device cluster,
      # so backward-fill and connected-components produce identical output.
      - {event_id: 1, device_id: 1, event_user_id: null, event_ts: '2026-04-01 10:00:00', event_date: '2026-04-01', event_name: 'page_view', platform: 'web', url: 'https://example.com/', session_id: 'sa', forward_only_user_id: 100, backward_fill_user_id: 100, connected_components_user_id: 100, connected_components_cluster_id: 100}
      - {event_id: 2, device_id: 1, event_user_id: 100,  event_ts: '2026-04-01 10:05:00', event_date: '2026-04-01', event_name: 'login',     platform: 'web', url: 'https://example.com/login', session_id: 'sa', forward_only_user_id: 100, backward_fill_user_id: 100, connected_components_user_id: 100, connected_components_cluster_id: 100}
      - {event_id: 3, device_id: 1, event_user_id: null, event_ts: '2026-04-01 10:08:00', event_date: '2026-04-01', event_name: 'page_view', platform: 'web', url: 'https://example.com/', session_id: 'sa', forward_only_user_id: null, backward_fill_user_id: 100, connected_components_user_id: 100, connected_components_cluster_id: 100}
      - {event_id: 4, device_id: 2, event_user_id: null, event_ts: '2026-04-01 11:00:00', event_date: '2026-04-01', event_name: 'page_view', platform: 'web', url: 'https://example.com/', session_id: 'sb', forward_only_user_id: null, backward_fill_user_id: null, connected_components_user_id: null, connected_components_cluster_id: null}
      # Day 2 (2026-04-02): 4 events on 2 devices, both signed-in. Devices 3
      # and 4 are in cluster {200, 201} with cluster_id 200, so
      # connected_components_user_id is 200 on every event of either device,
      # while backward_fill keeps them as 200 and 201 respectively. This is
      # the case that proves dau_connected_components < dau_backward_fill is
      # possible — DAU drops from 2 to 1 under cluster collapse.
      - {event_id: 5, device_id: 3, event_user_id: 200, event_ts: '2026-04-02 10:00:00', event_date: '2026-04-02', event_name: 'login',     platform: 'web', url: 'https://example.com/login', session_id: 'sc', forward_only_user_id: 200, backward_fill_user_id: 200, connected_components_user_id: 200, connected_components_cluster_id: 200}
      - {event_id: 6, device_id: 3, event_user_id: null, event_ts: '2026-04-02 10:05:00', event_date: '2026-04-02', event_name: 'page_view', platform: 'web', url: 'https://example.com/', session_id: 'sc', forward_only_user_id: 200, backward_fill_user_id: 200, connected_components_user_id: 200, connected_components_cluster_id: 200}
      - {event_id: 7, device_id: 4, event_user_id: 201, event_ts: '2026-04-02 11:00:00', event_date: '2026-04-02', event_name: 'login',     platform: 'web', url: 'https://example.com/login', session_id: 'sd', forward_only_user_id: 201, backward_fill_user_id: 201, connected_components_user_id: 200, connected_components_cluster_id: 200}
      - {event_id: 8, device_id: 4, event_user_id: null, event_ts: '2026-04-02 11:05:00', event_date: '2026-04-02', event_name: 'page_view', platform: 'web', url: 'https://example.com/', session_id: 'sd', forward_only_user_id: 201, backward_fill_user_id: 201, connected_components_user_id: 200, connected_components_cluster_id: 200}
      # Day 3 (2026-04-03): 2 events on 1 device, 1 signed-in (user 300) +
      # 1 anonymous. Forward-only identifies only the signed-in event;
      # backward-fill and connected-components retroactively tag the anonymous
      # event too. Singleton cluster, so dau is 1 for all three.
      - {event_id: 9,  device_id: 5, event_user_id: 300,  event_ts: '2026-04-03 10:00:00', event_date: '2026-04-03', event_name: 'login',     platform: 'web', url: 'https://example.com/login', session_id: 'se', forward_only_user_id: 300, backward_fill_user_id: 300, connected_components_user_id: 300, connected_components_cluster_id: 300}
      - {event_id: 10, device_id: 5, event_user_id: null, event_ts: '2026-04-03 10:05:00', event_date: '2026-04-03', event_name: 'page_view', platform: 'web', url: 'https://example.com/', session_id: 'se', forward_only_user_id: null, backward_fill_user_id: 300, connected_components_user_id: 300, connected_components_cluster_id: 300}
  expect:
    - {event_date: '2026-04-01', total_events: 4, dau_forward_only: 1, dau_backward_fill: 1, dau_connected_components: 1, identified_events_forward_only: 2, identified_events_backward_fill: 3, identified_events_connected_components: 3}
    - {event_date: '2026-04-02', total_events: 4, dau_forward_only: 2, dau_backward_fill: 2, dau_connected_components: 1, identified_events_forward_only: 4, identified_events_backward_fill: 4, identified_events_connected_components: 4}
    - {event_date: '2026-04-03', total_events: 2, dau_forward_only: 1, dau_backward_fill: 1, dau_connected_components: 1, identified_events_forward_only: 1, identified_events_backward_fill: 2, identified_events_connected_components: 2}
---
