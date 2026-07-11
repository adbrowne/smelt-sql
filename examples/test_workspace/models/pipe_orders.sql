-- @materialization: view
FROM smelt.raw_events
|> WHERE event_type = 'click'
|> SELECT user_id, event_time
|> ORDER BY event_time DESC
|> LIMIT 100
