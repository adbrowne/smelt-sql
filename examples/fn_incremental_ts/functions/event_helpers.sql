-- D1 fixture: simple function helpers for incremental models.

smelt.define is_peak_hour(event_ts: Expr<Timestamp>) -> Expr<Boolean> AS (
    EXTRACT(HOUR FROM event_ts) BETWEEN 9 AND 18
)

smelt.define hour_bucket(event_ts: Expr<Timestamp>) AS (
    CASE WHEN EXTRACT(HOUR FROM event_ts) BETWEEN 9 AND 18 THEN 'peak' ELSE 'off-peak' END
)
