-- Intentional parse error: missing FROM clause
SELECT user_id, COUNT(*)
WHERE active = true
GROUP BY user_id
