-- Lifetime value per user combining sessions and first purchase
SELECT
    s.user_id,
    fp.email,
    fp.plan_type,
    fp.first_purchase_time,
    COUNT(DISTINCT s.session_date) AS total_active_days,
    SUM(s.session_revenue_cents) AS lifetime_revenue_cents
FROM smelt.intermediate.user_sessions AS s
LEFT JOIN smelt.intermediate.user_first_purchase AS fp ON s.user_id = fp.user_id
GROUP BY s.user_id, fp.email, fp.plan_type, fp.first_purchase_time

