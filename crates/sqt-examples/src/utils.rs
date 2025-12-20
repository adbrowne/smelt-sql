use anyhow::Result;
use duckdb::Connection;
use arrow::util::pretty;

/// Helper to create a DuckDB connection with common setup
pub fn create_duckdb_connection() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    Ok(conn)
}

/// Execute a SQL statement and print results for debugging using Arrow
pub fn execute_and_print(conn: &Connection, sql: &str, description: &str) -> Result<()> {
    println!("\n=== {} ===", description);
    println!("SQL:\n{}\n", sql);

    // Execute query and get Arrow result
    let mut stmt = conn.prepare(sql)?;
    let arrow_result = stmt.query_arrow([])?;

    // Convert to RecordBatches and print
    let batches: Vec<_> = arrow_result.collect();

    // Use Arrow's pretty print
    pretty::print_batches(&batches)?;

    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    println!("\n({} rows)\n", total_rows);

    Ok(())
}
