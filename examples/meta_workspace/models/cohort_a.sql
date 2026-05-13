---
tags: [cohort, core]
---
-- Cohort model A: revenue events for the core user segment.
SELECT
    id,
    user_id,
    event_type,
    created_at
FROM smelt.sources.raw.events
WHERE event_type = 'revenue'
