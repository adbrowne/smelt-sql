/// Example 1: Common Intermediate Aggregation (Naive Version)
///
/// Scenario: Three analytics models that all need session-level data:
/// - Model A: Daily active sessions by user
/// - Model B: Session metrics by country
/// - Model C: Revenue per session hour-of-day
///
/// Naive approach: Each model computes sessions independently (3x redundant work)
///
/// A "session" is a group of events from the same user within a 30-minute window.

use anyhow::Result;
use duckdb::Connection;
use sqt_examples::utils::{create_duckdb_connection, execute_and_print};

fn setup_raw_events(conn: &Connection) -> Result<()> {
    // Create raw events table with sample data
    conn.execute_batch(
        "
        CREATE TABLE raw_events (
            event_id INTEGER,
            user_id INTEGER,
            event_time TIMESTAMP,
            country VARCHAR,
            event_type VARCHAR,
            revenue DECIMAL(10,2)
        );

        INSERT INTO raw_events VALUES
            -- User 1 sessions
            (1, 1, '2024-01-01 10:00:00', 'US', 'page_view', 0),
            (2, 1, '2024-01-01 10:05:00', 'US', 'click', 0),
            (3, 1, '2024-01-01 10:10:00', 'US', 'purchase', 50.00),
            -- Gap > 30 min, new session for User 1
            (4, 1, '2024-01-01 11:00:00', 'US', 'page_view', 0),
            (5, 1, '2024-01-01 11:15:00', 'US', 'purchase', 25.00),

            -- User 2 sessions
            (6, 2, '2024-01-01 09:30:00', 'UK', 'page_view', 0),
            (7, 2, '2024-01-01 09:45:00', 'UK', 'click', 0),
            (8, 2, '2024-01-01 10:00:00', 'UK', 'purchase', 100.00),
            -- New day, new session
            (9, 2, '2024-01-02 10:00:00', 'UK', 'page_view', 0),
            (10, 2, '2024-01-02 10:20:00', 'UK', 'purchase', 75.00),

            -- User 3 sessions (multiple countries)
            (11, 3, '2024-01-01 14:00:00', 'CA', 'page_view', 0),
            (12, 3, '2024-01-01 14:10:00', 'CA', 'purchase', 30.00),
            (13, 3, '2024-01-02 15:00:00', 'US', 'page_view', 0),
            (14, 3, '2024-01-02 15:20:00', 'US', 'purchase', 45.00);
        "
    )?;

    execute_and_print(conn, "SELECT * FROM raw_events ORDER BY user_id, event_time", "Raw Events Data")?;

    Ok(())
}

fn model_a_daily_active_sessions(conn: &Connection) -> Result<()> {
    // Model A: Daily active sessions per user
    // Each model independently computes sessions using window functions

    let sql = "
    WITH event_gaps AS (
        SELECT
            user_id,
            event_time,
            country,
            revenue,
            LAG(event_time) OVER (PARTITION BY user_id ORDER BY event_time) AS prev_event_time
        FROM raw_events
    ),
    sessions AS (
        SELECT
            user_id,
            event_time,
            country,
            revenue,
            -- Session ID: new session when gap > 30 minutes from previous event
            SUM(CASE
                WHEN prev_event_time IS NULL OR event_time - prev_event_time > INTERVAL 30 MINUTE
                THEN 1
                ELSE 0
            END) OVER (PARTITION BY user_id ORDER BY event_time) AS session_id
        FROM event_gaps
    ),
    session_summary AS (
        SELECT
            user_id,
            session_id,
            DATE_TRUNC('day', MIN(event_time)) AS session_day,
            COUNT(*) AS events_in_session,
            SUM(revenue) AS session_revenue
        FROM sessions
        GROUP BY user_id, session_id
    )
    SELECT
        session_day,
        user_id,
        COUNT(*) AS sessions_count,
        SUM(events_in_session) AS total_events,
        SUM(session_revenue) AS total_revenue
    FROM session_summary
    GROUP BY session_day, user_id
    ORDER BY session_day, user_id
    ";

    execute_and_print(conn, sql, "Model A: Daily Active Sessions by User")?;
    Ok(())
}

fn model_b_sessions_by_country(conn: &Connection) -> Result<()> {
    // Model B: Session metrics by country
    // REDUNDANT: Recomputes the same session aggregation

    let sql = "
    WITH event_gaps AS (
        SELECT
            user_id,
            event_time,
            country,
            revenue,
            LAG(event_time) OVER (PARTITION BY user_id ORDER BY event_time) AS prev_event_time
        FROM raw_events
    ),
    sessions AS (
        SELECT
            user_id,
            event_time,
            country,
            revenue,
            SUM(CASE
                WHEN prev_event_time IS NULL OR event_time - prev_event_time > INTERVAL 30 MINUTE
                THEN 1
                ELSE 0
            END) OVER (PARTITION BY user_id ORDER BY event_time) AS session_id
        FROM event_gaps
    ),
    session_summary AS (
        SELECT
            user_id,
            session_id,
            -- Take the first country seen in the session
            FIRST(country ORDER BY event_time) AS session_country,
            COUNT(*) AS events_in_session,
            SUM(revenue) AS session_revenue
        FROM sessions
        GROUP BY user_id, session_id
    )
    SELECT
        session_country,
        COUNT(*) AS sessions_count,
        SUM(events_in_session) AS total_events,
        SUM(session_revenue) AS total_revenue,
        AVG(session_revenue) AS avg_revenue_per_session
    FROM session_summary
    GROUP BY session_country
    ORDER BY session_country
    ";

    execute_and_print(conn, sql, "Model B: Sessions by Country")?;
    Ok(())
}

fn model_c_revenue_by_hour_of_day(conn: &Connection) -> Result<()> {
    // Model C: Revenue per session by hour-of-day
    // REDUNDANT: Recomputes the same session aggregation AGAIN

    let sql = "
    WITH event_gaps AS (
        SELECT
            user_id,
            event_time,
            country,
            revenue,
            LAG(event_time) OVER (PARTITION BY user_id ORDER BY event_time) AS prev_event_time
        FROM raw_events
    ),
    sessions AS (
        SELECT
            user_id,
            event_time,
            country,
            revenue,
            SUM(CASE
                WHEN prev_event_time IS NULL OR event_time - prev_event_time > INTERVAL 30 MINUTE
                THEN 1
                ELSE 0
            END) OVER (PARTITION BY user_id ORDER BY event_time) AS session_id
        FROM event_gaps
    ),
    session_summary AS (
        SELECT
            user_id,
            session_id,
            EXTRACT(HOUR FROM MIN(event_time)) AS session_hour,
            COUNT(*) AS events_in_session,
            SUM(revenue) AS session_revenue
        FROM sessions
        GROUP BY user_id, session_id
    )
    SELECT
        session_hour,
        COUNT(*) AS sessions_count,
        SUM(session_revenue) AS total_revenue,
        AVG(session_revenue) AS avg_revenue_per_session
    FROM session_summary
    GROUP BY session_hour
    ORDER BY session_hour
    ";

    execute_and_print(conn, sql, "Model C: Revenue by Session Hour-of-Day")?;
    Ok(())
}

fn main() -> Result<()> {
    println!("\n╔════════════════════════════════════════════════════════════╗");
    println!("║  Example 1: Common Intermediate Aggregation (NAIVE)       ║");
    println!("╚════════════════════════════════════════════════════════════╝\n");

    println!("Problem: All three models compute sessions independently.");
    println!("The session logic (30-min window) is repeated 3 times!\n");

    let conn = create_duckdb_connection()?;

    // Setup data
    setup_raw_events(&conn)?;

    // Run all three models (each recomputes sessions)
    model_a_daily_active_sessions(&conn)?;
    model_b_sessions_by_country(&conn)?;
    model_c_revenue_by_hour_of_day(&conn)?;

    println!("\n╔════════════════════════════════════════════════════════════╗");
    println!("║  Analysis: What's Redundant?                               ║");
    println!("╚════════════════════════════════════════════════════════════╝\n");
    println!("All three models have identical logic for:");
    println!("  1. Computing session_id (30-minute window detection)");
    println!("  2. Initial GROUP BY (user_id, session_id)");
    println!("\nThey only differ in:");
    println!("  - Model A: Final GROUP BY (day, user_id)");
    println!("  - Model B: Final GROUP BY (country)");
    println!("  - Model C: Final GROUP BY (hour)");
    println!("\nOptimization opportunity:");
    println!("  → Compute session_summary ONCE");
    println!("  → Derive all three models from it\n");

    Ok(())
}
