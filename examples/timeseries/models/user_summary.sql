---
name: user_summary
materialization: table
tags: [users, analytics]
owner: data-team
description: User event summary with first and last event times
---

-- Example showing YAML frontmatter metadata
SELECT
    user_id,
    MIN(event_timestamp) AS first_event_timestamp,
    MAX(event_timestamp) AS last_event_timestamp
FROM smelt.source('raw.events')
GROUP BY user_id