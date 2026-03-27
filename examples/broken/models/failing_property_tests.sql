-- Models with tests that demonstrate various property-based test failures.

-- Simple model used as target for tests with wrong expected values.
--- name: date_arithmetic ---
---
WITH events AS (
    SELECT
        event_id,
        user_id,
        event_type,
        event_date,
        amount
    FROM raw_events
),
daily_stats AS (
    SELECT
        event_date AS day,
        event_type,
        COUNT(*) AS event_count,
        SUM(amount) AS total_amount
    FROM events
    GROUP BY event_date, event_type
)
SELECT * FROM daily_stats

--- name: test_property_wrong_aggregation ---
materialization: test
test:
  model: date_arithmetic
  target_cte: daily_stats
  cases: 5
  inputs:
    events:
      - {event_date: '2024-01-01', event_type: click, amount: 10.0}
      - {event_date: '2024-01-01', event_type: click, amount: 20.0}
      - {event_date: '2024-01-01', event_type: purchase, amount: 100.0}
  expect:
    - {day: '2024-01-01', event_type: click, event_count: 2, total_amount: 999.0}
    - {day: '2024-01-01', event_type: purchase, event_count: 1, total_amount: 100.0}
---

--- name: test_property_missing_row ---
materialization: test
test:
  model: date_arithmetic
  target_cte: daily_stats
  cases: 5
  inputs:
    events:
      - {event_date: '2024-01-01', event_type: click, amount: 10.0}
      - {event_date: '2024-01-02', event_type: click, amount: 20.0}
  expect:
    - {day: '2024-01-01', event_type: click, event_count: 1, total_amount: 10.0}
---

-- Demonstrates property test catching NULL propagation bugs.
-- The concat `first_name || ' ' || last_name` returns NULL when last_name is NULL.
-- Since last_name is omitted from inputs, the framework generates random values
-- including NULLs (~10% chance per iteration). When last_name is NULL, full_name
-- becomes NULL instead of a non-null string, failing the non-null expectation.
--- name: null_sensitive_model ---
---
WITH users AS (
    SELECT user_id, first_name, last_name FROM raw_users
),
formatted AS (
    SELECT
        user_id,
        first_name || ' ' || last_name AS full_name
    FROM users
    WHERE first_name || ' ' || last_name IS NOT NULL
)
SELECT * FROM formatted

--- name: test_property_null_in_concat ---
materialization: test
test:
  model: null_sensitive_model
  target_cte: formatted
  cases: 30
  inputs:
    users:
      - {user_id: 1, first_name: Alice}
      - {user_id: 2, first_name: Bob}
  expect:
    - {user_id: 1}
    - {user_id: 2}
---

-- Demonstrates property test catching type mismatch errors.
-- The model divides by num_items, but when num_items is omitted from inputs
-- the framework generates Text values (type not inferable), and DuckDB rejects
-- the division with a type error.
--- name: division_model ---
---
WITH metrics AS (
    SELECT category, total_sales, num_items FROM raw_metrics
),
averages AS (
    SELECT
        category,
        total_sales / num_items AS avg_price
    FROM metrics
)
SELECT * FROM averages

--- name: test_property_type_mismatch ---
materialization: test
test:
  model: division_model
  target_cte: averages
  cases: 5
  inputs:
    metrics:
      - {category: electronics, total_sales: 1000.0}
      - {category: books, total_sales: 250.0}
  expect:
    - {category: electronics}
    - {category: books}
---

-- Demonstrates property test finding unhandled CASE branches.
-- When status and priority are both omitted, the framework generates random
-- strings for status that won't match any CASE branch, producing NULL status_label.
-- The test expects non-null values, so it fails.
--- name: status_classifier ---
---
WITH tickets AS (
    SELECT ticket_id, status, priority FROM raw_tickets
),
classified AS (
    SELECT
        ticket_id,
        CASE status
            WHEN 'open' THEN 'active'
            WHEN 'closed' THEN 'resolved'
            WHEN 'pending' THEN 'waiting'
        END AS status_label
    FROM tickets
)
SELECT * FROM classified

--- name: test_property_unhandled_case ---
materialization: test
test:
  model: status_classifier
  target_cte: classified
  cases: 20
  inputs:
    tickets:
      - {ticket_id: 1}
      - {ticket_id: 2}
  expect:
    - {ticket_id: 1, status_label: active}
    - {ticket_id: 2, status_label: resolved}
---
