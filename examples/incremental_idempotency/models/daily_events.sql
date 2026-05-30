-- Incremental daily aggregate. partition_column (event_date) is projected and
-- grouped, satisfying the SELECT+GROUP BY requirement. The framework injects
-- the run-window time filter before DELETE+INSERT.
SELECT
    CAST(event_ts AS DATE) AS event_date,
    user_id,
    COUNT(*) AS event_count
FROM smelt.sources.raw.pulse
GROUP BY 1, 2
