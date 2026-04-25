-- Phase 37 fixture: calls with_hour and projects the result struct.
SELECT smelt.fn.with_hour(e) AS ev
FROM smelt.source('source.events') AS e
