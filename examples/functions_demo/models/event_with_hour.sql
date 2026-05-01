-- Phase 37 fixture: calls with_hour and projects the result struct.
SELECT smelt.functions.with_hour(e) AS ev
FROM smelt.sources.source.events AS e

