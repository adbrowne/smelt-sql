/// Example 1: Common Intermediate Aggregation (Optimized Version)
///
/// This version computes the session_summary CTE ONCE and reuses it for all three models.
/// This demonstrates what an optimizer should automatically detect and generate.

use anyhow::Result;
use duckdb::Connection;
use sqt_examples::utils::{create_duckdb_connection, execute_and_print};

fn setup_raw_events(conn: &Connection) -> Result<()> {
    // Same data as naive version
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

    Ok(())
}

fn create_shared_session_summary(conn: &Connection) -> Result<()> {
    // Compute session summary ONCE as a materialized view or temp table
    // This is what the optimizer should automatically generate

    let sql = "
    CREATE TEMP TABLE session_summary AS
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
    )
    SELECT
        user_id,
        session_id,
        MIN(event_time) AS session_start_time,
        DATE_TRUNC('day', MIN(event_time)) AS session_day,
        EXTRACT(HOUR FROM MIN(event_time)) AS session_hour,
        FIRST(country ORDER BY event_time) AS session_country,
        COUNT(*) AS events_in_session,
        SUM(revenue) AS session_revenue
    FROM sessions
    GROUP BY user_id, session_id
    ";

    conn.execute(sql, [])?;
    execute_and_print(conn, "SELECT * FROM session_summary ORDER BY user_id, session_id", "Shared Session Summary (Computed Once)")?;

    Ok(())
}

fn model_a_daily_active_sessions(conn: &Connection) -> Result<()> {
    // Model A: Now simply queries the shared session_summary table

    let sql = "
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

    execute_and_print(conn, sql, "Model A: Daily Active Sessions (From Shared Summary)")?;
    Ok(())
}

fn model_b_sessions_by_country(conn: &Connection) -> Result<()> {
    // Model B: Queries the same shared session_summary table

    let sql = "
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

    execute_and_print(conn, sql, "Model B: Sessions by Country (From Shared Summary)")?;
    Ok(())
}

fn model_c_revenue_by_hour_of_day(conn: &Connection) -> Result<()> {
    // Model C: Also queries the shared session_summary table

    let sql = "
    SELECT
        session_hour,
        COUNT(*) AS sessions_count,
        SUM(session_revenue) AS total_revenue,
        AVG(session_revenue) AS avg_revenue_per_session
    FROM session_summary
    GROUP BY session_hour
    ORDER BY session_hour
    ";

    execute_and_print(conn, sql, "Model C: Revenue by Hour (From Shared Summary)")?;
    Ok(())
}

fn main() -> Result<()> {
    println!("\n╔════════════════════════════════════════════════════════════╗");
    println!("║  Example 1: Common Intermediate Aggregation (OPTIMIZED)   ║");
    println!("╚════════════════════════════════════════════════════════════╝\n");

    println!("Optimization: Compute session_summary ONCE, reuse for all models.\n");

    let conn = create_duckdb_connection()?;

    // Setup data
    setup_raw_events(&conn)?;

    // Create the shared intermediate materialization
    create_shared_session_summary(&conn)?;

    // All three models now query from the shared summary
    model_a_daily_active_sessions(&conn)?;
    model_b_sessions_by_country(&conn)?;
    model_c_revenue_by_hour_of_day(&conn)?;

    println!("\n╔════════════════════════════════════════════════════════════╗");
    println!("║  Key Insights for Optimizer API Design                    ║");
    println!("╚════════════════════════════════════════════════════════════╝\n");
    println!("1. Pattern Detection:");
    println!("   - Need to detect common subexpressions across models");
    println!("   - The 'event_gaps' and 'sessions' CTEs are identical");
    println!("   - The initial GROUP BY (user_id, session_id) is identical\n");

    println!("2. Materialization Decision:");
    println!("   - Should session_summary be a temp table, view, or CTE?");
    println!("   - Depends on: data size, reuse count, backend capabilities");
    println!("   - In this case: temp table makes sense (3 consumers)\n");

    println!("3. Schema Requirements:");
    println!("   - session_summary needs ALL dimensions used by consumers");
    println!("   - Must include: day, hour, country (even if not in original logic)");
    println!("   - Optimizer must compute the 'union' of required dimensions\n");

    println!("4. Correctness Preservation:");
    println!("   - Results must be identical to naive version");
    println!("   - Run example1_naive to compare outputs\n");

    println!("5. API Design Question:");
    println!("   - How does a data engineer specify this optimization?");
    println!("   - Option A: Automatic detection (no user input needed)");
    println!("   - Option B: Hint/annotation (e.g., @materialize(sessions))");
    println!("   - Option C: Explicit rule (pattern match + rewrite)\n");

    Ok(())
}
