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
    first_value(event_time) over (PARTITION BY user_id ORDER BY event_time) AS first_event_time,
    last_value(event_time) over (PARTITION BY user_id ORDER BY event_time) AS last_event_time
FROM smelt.source('raw.events')
GROUP BY user_id