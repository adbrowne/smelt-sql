-- Extract structured fields from JSON event properties
--
-- Demonstrates smelt's canonical JSON functions:
--   to_json, json_extract / ->, json_extract_text / ->>
-- These are rewritten to the target engine's syntax at execution time.
SELECT
    e.event_id,
    e.user_id,
    e.event_type,
    e.event_timestamp,
    e.properties ->> 'page_url' AS page_url,
    e.properties ->> 'referrer' AS referrer,
    CAST(e.properties ->> 'duration_ms' AS INTEGER) AS duration_ms,
    e.properties -> 'metadata' AS metadata_json,
    json_array_length(e.properties -> 'tags') AS tag_count
FROM smelt.ref('events') e
WHERE e.properties IS NOT NULL
