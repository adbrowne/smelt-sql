---
tags:
  - event_source
---
SELECT event_id, user_id, event_time, event_type
FROM smelt.models.raw_events
WHERE event_type = 'page_view'

