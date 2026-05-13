---
tags: [cohort]
---
-- Cohort model C: page-view events for the cohort segment.
SELECT
    id,
    user_id,
    event_type,
    created_at
FROM smelt.sources.raw.events
WHERE event_type = 'pageview'
