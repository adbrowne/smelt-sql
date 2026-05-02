-- Events with epoch-based gap calculation (bug #4 EXTRACT EPOCH)
SELECT
    event_id,
    visitor_id,
    event_type,
    event_timestamp,
    page_url,
    EXTRACT(EPOCH FROM event_timestamp) - EXTRACT(EPOCH FROM LAG(event_timestamp) OVER (PARTITION BY visitor_id ORDER BY event_timestamp)) AS gap_seconds
FROM smelt.sources.raw.events

