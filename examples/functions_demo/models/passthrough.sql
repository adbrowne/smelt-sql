-- A minimal passthrough model over the events source. Subsequent phases
-- extend this workspace with models that call `smelt.fn.*` functions.
SELECT
    event_id,
    user_id,
    event_type
FROM smelt.source('source.events')
