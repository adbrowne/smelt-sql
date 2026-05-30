---
materialization: table
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
-- Self-contained driving source (no external seed): three day-partitions.
SELECT * FROM (
    VALUES
        (DATE '2026-01-01', 1, 100, 5),
        (DATE '2026-01-01', 1, 100, 3),
        (DATE '2026-01-01', 2, 200, 8),
        (DATE '2026-01-02', 1, 100, 7),
        (DATE '2026-01-02', 2, 200, 1),
        (DATE '2026-01-02', 3, 300, 9),
        (DATE '2026-01-03', 1, 100, 2),
        (DATE '2026-01-03', 2, 200, 6),
        (DATE '2026-01-03', 3, 300, 4)
) AS t(event_date, device_id, user_id, amount)
