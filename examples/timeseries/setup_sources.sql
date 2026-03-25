-- Setup script to create source tables and populate with test data

-- Create raw schema
CREATE SCHEMA IF NOT EXISTS raw;

-- Create and populate users table
CREATE TABLE IF NOT EXISTS raw.users (
    user_id INTEGER,
    user_name VARCHAR,
    signup_date DATE
);

DELETE FROM raw.users;

INSERT INTO raw.users VALUES
    (1, 'Alice', '2025-01-01'),
    (2, 'Bob', '2025-01-02'),
    (3, 'Charlie', '2025-01-03'),
    (4, 'Diana', '2025-01-04'),
    (5, 'Eve', '2025-01-05');

-- Create and populate events table
CREATE TABLE IF NOT EXISTS raw.events (
    event_id INTEGER,
    user_id INTEGER,
    event_type VARCHAR,
    event_timestamp TIMESTAMP
);

DELETE FROM raw.events;

INSERT INTO raw.events VALUES
    (1, 1, 'login', '2025-01-10 08:00:00'),
    (2, 1, 'page_view', '2025-01-10 08:05:00'),
    (3, 2, 'login', '2025-01-10 09:00:00'),
    (4, 1, 'purchase', '2025-01-10 10:30:00'),
    (5, 3, 'login', '2025-01-10 11:00:00'),
    (6, 2, 'page_view', '2025-01-10 11:15:00'),
    (7, 3, 'page_view', '2025-01-10 11:20:00'),
    (8, 1, 'logout', '2025-01-10 12:00:00'),
    (9, 4, 'login', '2025-01-10 13:00:00'),
    (10, 2, 'purchase', '2025-01-10 14:00:00');

-- Create and populate transactions table
CREATE TABLE IF NOT EXISTS raw.transactions (
    transaction_id INTEGER,
    user_id INTEGER,
    amount DECIMAL(10,2),
    transaction_timestamp TIMESTAMP,
    transaction_type VARCHAR
);

DELETE FROM raw.transactions;

-- Insert sample data spanning 30 days
INSERT INTO raw.transactions
WITH dates AS (
    SELECT TIMESTAMP '2024-01-01 00:00:00' + INTERVAL (d) DAY
           + INTERVAL (CAST(RANDOM() * 24 AS INTEGER)) HOUR
           + INTERVAL (CAST(RANDOM() * 60 AS INTEGER)) MINUTE AS ts
    FROM UNNEST(RANGE(0, 30)) AS t(d)
),
users AS (
    SELECT UNNEST(RANGE(1, 101)) AS user_id
)
SELECT
    ROW_NUMBER() OVER () AS transaction_id,
    user_id,
    (RANDOM() * 100 + 10)::DECIMAL(10,2) AS amount,
    ts AS transaction_timestamp,
    CASE (RANDOM() * 4)::INTEGER
        WHEN 0 THEN 'purchase'
        WHEN 1 THEN 'refund'
        WHEN 2 THEN 'subscription'
        ELSE 'tip'
    END AS transaction_type
FROM dates
CROSS JOIN users
WHERE RANDOM() < 0.05  -- 5% sampling
LIMIT 10000;
