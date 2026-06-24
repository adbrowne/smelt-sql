--- name: test_backward_fill_resolution_invariants ---
materialization: test
test:
  model: identity_backward_fill
  inputs:
    silver.device_user_edges:
      # Device 1 — clear winner on event_count (user 100 wins: 5 > 2)
      - {device_id: 1, user_id: 100, event_count: 5, first_seen: '2026-04-01 10:00:00', last_seen: '2026-04-01 10:30:00'}
      - {device_id: 1, user_id: 101, event_count: 2, first_seen: '2026-04-01 11:00:00', last_seen: '2026-04-01 11:10:00'}
      # Device 2 — tie on event_count=3, broken by earlier first_seen (user 201 wins: 11:00 < 12:00)
      - {device_id: 2, user_id: 200, event_count: 3, first_seen: '2026-04-01 12:00:00', last_seen: '2026-04-01 12:30:00'}
      - {device_id: 2, user_id: 201, event_count: 3, first_seen: '2026-04-01 11:00:00', last_seen: '2026-04-01 11:30:00'}
      # Device 3 — single candidate
      - {device_id: 3, user_id: 300, event_count: 1, first_seen: '2026-04-01 13:00:00', last_seen: '2026-04-01 13:01:00'}
      # Device 4 — three-way: 400 and 401 tie on event_count=10; 401 wins (earlier first_seen 08:00 < 09:00).
      # 402 has the earliest first_seen overall (07:00) but loses on primary sort (event_count=5 < 10).
      - {device_id: 4, user_id: 400, event_count: 10, first_seen: '2026-04-01 09:00:00', last_seen: '2026-04-01 12:00:00'}
      - {device_id: 4, user_id: 401, event_count: 10, first_seen: '2026-04-01 08:00:00', last_seen: '2026-04-01 11:00:00'}
      - {device_id: 4, user_id: 402, event_count: 5,  first_seen: '2026-04-01 07:00:00', last_seen: '2026-04-01 07:30:00'}
  expect:
    - {device_id: 1, backward_fill_amplitude_id: 'u:100'}  # higher event_count wins
    - {device_id: 2, backward_fill_amplitude_id: 'u:201'}  # tie broken by earlier first_seen
    - {device_id: 3, backward_fill_amplitude_id: 'u:300'}  # only candidate
    - {device_id: 4, backward_fill_amplitude_id: 'u:401'}  # primary sort dominates: 401 wins the event_count tie
---
