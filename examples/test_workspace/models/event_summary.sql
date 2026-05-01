SELECT event_type, COUNT(*) as event_count
FROM smelt.models.combined_events
GROUP BY event_type

