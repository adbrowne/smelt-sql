-- First purchase event per user
WITH purchases AS (
    SELECT
        e.user_id,
        e.event_time AS purchase_time,
        e.revenue_cents,
        ROW_NUMBER() OVER (PARTITION BY e.user_id ORDER BY e.event_time) AS rn
    FROM smelt.models.staging.stg_events AS e
    WHERE e.event_type = 'purchase'
)
SELECT
    p.user_id,
    u.email,
    u.plan_type,
    p.purchase_time AS first_purchase_time,
    p.revenue_cents AS first_purchase_cents
FROM purchases AS p
INNER JOIN smelt.models.staging.stg_users AS u ON p.user_id = u.user_id
WHERE p.rn = 1

