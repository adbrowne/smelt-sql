-- Simulated raw orders table
SELECT 1 AS order_id, 100 AS user_id, 29.99 AS amount, 'completed' AS status, DATE '2025-01-15' AS created_at
UNION ALL SELECT 2, 101, 49.99, 'completed', DATE '2025-01-15'
UNION ALL SELECT 3, 100, 15.00, 'cancelled', DATE '2025-01-16'
UNION ALL SELECT 4, 102, 75.50, 'completed', DATE '2025-01-16'
UNION ALL SELECT 5, 101, 22.00, 'pending', DATE '2025-01-17'

