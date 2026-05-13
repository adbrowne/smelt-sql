---
tags: [cohort, audit]
---
-- Cohort model B: sign-up events for the audit segment.
SELECT
    id,
    user_id,
    event_type,
    created_at
FROM smelt.sources.raw.events
WHERE event_type = 'signup'
