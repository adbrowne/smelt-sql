--- name: test_dau_monotonicity_invariants ---
materialization: test
test:
  model: daily_active_users_by_method
  inputs:
    gold.eventstream_with_identity:
      # Day 1 (2026-04-01): 4 events on 2 devices. Exercises the four-way
      # monotonicity invariants on a per-day row:
      #   identified_events_raw ≤ identified_events_forward_only
      #     ≤ identified_events_backward_fill = identified_events_connected_components
      #   dau_raw ≥ dau_forward_only ≥ dau_backward_fill ≥ dau_connected_components
      # Device 1 has one signed-in event (user 100); backward_fill back-tags every
      # event on device 1 to 'u:100'. Device 2 stays anonymous everywhere, so it
      # resolves to 'd:2' under every method.
      - {event_id: 1, device_id: 1, event_user_id: null, amplitude_id: 'd:1',   event_ts: '2026-04-01 10:00:00', event_date: '2026-04-01', event_name: 'page_view', platform: 'web', url: 'https://example.com/',      session_id: 'sa', forward_only_amplitude_id: 'u:100', backward_fill_amplitude_id: 'u:100', connected_components_amplitude_id: 'u:100', connected_components_cluster_id: 'u:100'}
      - {event_id: 2, device_id: 1, event_user_id: 100,  amplitude_id: 'u:100', event_ts: '2026-04-01 10:05:00', event_date: '2026-04-01', event_name: 'login',     platform: 'web', url: 'https://example.com/login', session_id: 'sa', forward_only_amplitude_id: 'u:100', backward_fill_amplitude_id: 'u:100', connected_components_amplitude_id: 'u:100', connected_components_cluster_id: 'u:100'}
      # Event 3 mocks an event outside the upstream window-function tag: its
      # forward_only_amplitude_id falls back to the device prefix at the
      # eventstream COALESCE boundary.
      - {event_id: 3, device_id: 1, event_user_id: null, amplitude_id: 'd:1',   event_ts: '2026-04-01 10:08:00', event_date: '2026-04-01', event_name: 'page_view', platform: 'web', url: 'https://example.com/',      session_id: 'sa', forward_only_amplitude_id: 'd:1',   backward_fill_amplitude_id: 'u:100', connected_components_amplitude_id: 'u:100', connected_components_cluster_id: 'u:100'}
      - {event_id: 4, device_id: 2, event_user_id: null, amplitude_id: 'd:2',   event_ts: '2026-04-01 11:00:00', event_date: '2026-04-01', event_name: 'page_view', platform: 'web', url: 'https://example.com/',      session_id: 'sb', forward_only_amplitude_id: 'd:2',   backward_fill_amplitude_id: 'd:2',   connected_components_amplitude_id: 'd:2',   connected_components_cluster_id: 'd:2'}
      # Day 2 (2026-04-02): 4 events on 2 devices, both signed-in. Devices 3
      # and 4 are in cluster {200, 201} with representative 'u:200', so
      # connected_components_amplitude_id is 'u:200' on every event of either device,
      # while backward_fill keeps them as 'u:200' and 'u:201' respectively. This
      # is the case that proves dau_connected_components < dau_backward_fill is
      # possible — DAU drops from 2 to 1 under cluster collapse.
      - {event_id: 5, device_id: 3, event_user_id: 200, amplitude_id: 'u:200', event_ts: '2026-04-02 10:00:00', event_date: '2026-04-02', event_name: 'login',     platform: 'web', url: 'https://example.com/login', session_id: 'sc', forward_only_amplitude_id: 'u:200', backward_fill_amplitude_id: 'u:200', connected_components_amplitude_id: 'u:200', connected_components_cluster_id: 'u:200'}
      - {event_id: 6, device_id: 3, event_user_id: null, amplitude_id: 'd:3',  event_ts: '2026-04-02 10:05:00', event_date: '2026-04-02', event_name: 'page_view', platform: 'web', url: 'https://example.com/',      session_id: 'sc', forward_only_amplitude_id: 'u:200', backward_fill_amplitude_id: 'u:200', connected_components_amplitude_id: 'u:200', connected_components_cluster_id: 'u:200'}
      - {event_id: 7, device_id: 4, event_user_id: 201, amplitude_id: 'u:201', event_ts: '2026-04-02 11:00:00', event_date: '2026-04-02', event_name: 'login',     platform: 'web', url: 'https://example.com/login', session_id: 'sd', forward_only_amplitude_id: 'u:201', backward_fill_amplitude_id: 'u:201', connected_components_amplitude_id: 'u:200', connected_components_cluster_id: 'u:200'}
      - {event_id: 8, device_id: 4, event_user_id: null, amplitude_id: 'd:4',  event_ts: '2026-04-02 11:05:00', event_date: '2026-04-02', event_name: 'page_view', platform: 'web', url: 'https://example.com/',      session_id: 'sd', forward_only_amplitude_id: 'u:201', backward_fill_amplitude_id: 'u:201', connected_components_amplitude_id: 'u:200', connected_components_cluster_id: 'u:200'}
      # Day 3 (2026-04-03): 2 events on 1 device, 1 signed-in (user 300) +
      # 1 anonymous. Forward-only identifies only the signed-in event;
      # backward-fill and connected-components retroactively tag the anonymous
      # event too. Singleton cluster.
      - {event_id: 9,  device_id: 5, event_user_id: 300,  amplitude_id: 'u:300', event_ts: '2026-04-03 10:00:00', event_date: '2026-04-03', event_name: 'login',     platform: 'web', url: 'https://example.com/login', session_id: 'se', forward_only_amplitude_id: 'u:300', backward_fill_amplitude_id: 'u:300', connected_components_amplitude_id: 'u:300', connected_components_cluster_id: 'u:300'}
      - {event_id: 10, device_id: 5, event_user_id: null, amplitude_id: 'd:5',  event_ts: '2026-04-03 10:05:00', event_date: '2026-04-03', event_name: 'page_view', platform: 'web', url: 'https://example.com/',      session_id: 'se', forward_only_amplitude_id: 'd:5',   backward_fill_amplitude_id: 'u:300', connected_components_amplitude_id: 'u:300', connected_components_cluster_id: 'u:300'}
  expect:
    # Day 1: raw distinct = {'d:1', 'u:100', 'd:2'} = 3
    #        forward_only distinct = {'u:100', 'd:1', 'd:2'} = 3
    #        backward_fill distinct = {'u:100', 'd:2'} = 2
    #        connected_components distinct = {'u:100', 'd:2'} = 2
    #        identified_raw = 1 (ev 2 only)
    #        identified_forward_only = 2 (ev 1, 2)
    #        identified_backward_fill = 3 (ev 1, 2, 3)
    #        identified_connected_components = 3
    - {event_date: '2026-04-01', total_events: 4, dau_raw: 3, dau_forward_only: 3, dau_backward_fill: 2, dau_connected_components: 2, identified_events_raw: 1, identified_events_forward_only: 2, identified_events_backward_fill: 3, identified_events_connected_components: 3}
    # Day 2: raw distinct = {'u:200', 'd:3', 'u:201', 'd:4'} = 4
    #        forward_only distinct = {'u:200', 'u:201'} = 2
    #        backward_fill distinct = {'u:200', 'u:201'} = 2
    #        connected_components distinct = {'u:200'} = 1 (cluster collapse visible)
    #        identified_raw = 2; identified_forward = 4; identified_backward = 4; identified_connected = 4
    - {event_date: '2026-04-02', total_events: 4, dau_raw: 4, dau_forward_only: 2, dau_backward_fill: 2, dau_connected_components: 1, identified_events_raw: 2, identified_events_forward_only: 4, identified_events_backward_fill: 4, identified_events_connected_components: 4}
    # Day 3: raw distinct = {'u:300', 'd:5'} = 2
    #        forward_only distinct = {'u:300', 'd:5'} = 2
    #        backward_fill distinct = {'u:300'} = 1
    #        connected_components distinct = {'u:300'} = 1
    #        identified_raw = 1; identified_forward = 1; identified_backward = 2; identified_connected = 2
    - {event_date: '2026-04-03', total_events: 2, dau_raw: 2, dau_forward_only: 2, dau_backward_fill: 1, dau_connected_components: 1, identified_events_raw: 1, identified_events_forward_only: 1, identified_events_backward_fill: 2, identified_events_connected_components: 2}
---
