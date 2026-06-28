-- Tests that intentionally fail, demonstrating various failure modes.
-- All tests below should fail when run with `smelt test`.

-- Intentional failure: wrong expected total_amount for 'click' (999.0 vs actual 30.0).
smelt.test test_property_wrong_aggregation AS (
    SELECT event_date AS day, event_type, COUNT(*) AS event_count, SUM(amount) AS total_amount
    FROM smelt.events
    GROUP BY event_date, event_type
)
PASSING events AS (
    {event_date: '2024-01-01', event_type: 'click',    amount: 10.0},
    {event_date: '2024-01-01', event_type: 'click',    amount: 20.0},
    {event_date: '2024-01-01', event_type: 'purchase', amount: 100.0}
)
EXPECT (
    {day: '2024-01-01', event_type: 'click',    event_count: 2, total_amount: 999.0},
    {day: '2024-01-01', event_type: 'purchase', event_count: 1, total_amount: 100.0}
)

-- Intentional failure: only one expected row but two rows are produced.
smelt.test test_property_missing_row AS (
    SELECT event_date AS day, event_type, COUNT(*) AS event_count, SUM(amount) AS total_amount
    FROM smelt.events
    GROUP BY event_date, event_type
)
PASSING events AS (
    {event_date: '2024-01-01', event_type: 'click', amount: 10.0},
    {event_date: '2024-01-02', event_type: 'click', amount: 20.0}
)
EXPECT (
    {day: '2024-01-01', event_type: 'click', event_count: 1, total_amount: 10.0}
)

-- Intentional failure: last_name IS NULL causes the WHERE filter to exclude all rows,
-- so actual has 0 rows but expected has 2.
smelt.test test_property_null_in_concat AS (
    SELECT
        user_id,
        first_name || ' ' || last_name AS full_name
    FROM smelt.users
    WHERE first_name || ' ' || last_name IS NOT NULL
)
PASSING users AS (
    {user_id: 1, first_name: 'Alice', last_name: null},
    {user_id: 2, first_name: 'Bob',   last_name: null}
)
EXPECT (
    {user_id: 1},
    {user_id: 2}
)

-- Intentional failure: wrong expected avg_price values (actual 100 and 50, not 999).
smelt.test test_property_type_mismatch AS (
    SELECT category, total_sales / num_items AS avg_price
    FROM smelt.metrics
)
PASSING metrics AS (
    {category: 'electronics', total_sales: 1000.0, num_items: 10},
    {category: 'books',       total_sales: 250.0,  num_items: 5}
)
EXPECT (
    {category: 'electronics', avg_price: 999.0},
    {category: 'books',       avg_price: 999.0}
)

-- Intentional failure: unhandled CASE branch produces NULL status_label but expected
-- non-null values 'active' and 'resolved'.
smelt.test test_property_unhandled_case AS (
    SELECT
        ticket_id,
        CASE status
            WHEN 'open'    THEN 'active'
            WHEN 'closed'  THEN 'resolved'
            WHEN 'pending' THEN 'waiting'
        END AS status_label
    FROM smelt.tickets
)
PASSING tickets AS (
    {ticket_id: 1, status: 'other'},
    {ticket_id: 2, status: 'unknown'}
)
EXPECT (
    {ticket_id: 1, status_label: 'active'},
    {ticket_id: 2, status_label: 'resolved'}
)
