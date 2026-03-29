#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Colors for output
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

info()  { echo -e "${BLUE}[INFO]${NC} $*"; }
step()  { echo -e "\n${GREEN}=== $* ===${NC}"; }
warn()  { echo -e "${YELLOW}[WARN]${NC} $*"; }

echo -e "${GREEN}"
echo "========================================"
echo "  Multi-Engine Pipeline Demo"
echo "========================================"
echo -e "${NC}"
echo "This example demonstrates cross-engine execution:"
echo "  datagen -> Spark (aggregate) -> DuckDB (metrics)"
echo ""
echo "Mode: LOCAL (DuckDB simulates Spark output)"
echo ""

# Determine SMELT and DATAGEN commands
SMELT="cargo run -p smelt-cli --manifest-path $PROJECT_ROOT/Cargo.toml --"
DATAGEN="cargo run -p smelt-datagen --manifest-path $PROJECT_ROOT/Cargo.toml --"

# Paths (relative to SCRIPT_DIR, which is where smelt.yml lives)
DATA_DIR="$SCRIPT_DIR/data/sessions"
SPARK_WAREHOUSE="$SCRIPT_DIR/target/spark/analytics/int_visitor_daily"
DUCKDB_PATH="$SCRIPT_DIR/target/warehouse.duckdb"

# Clean up previous state
step "Step 0: Clean previous state"
rm -rf "$SCRIPT_DIR/data" "$SCRIPT_DIR/target"
info "Cleaned data/ and target/"

# ---------------------------------------------------------------
# Step 1: Generate all raw data (14 days)
# ---------------------------------------------------------------
step "Step 1: Generate raw clickstream data (14 days)"
(cd "$SCRIPT_DIR" && $DATAGEN --config datagen.yaml --quiet)
info "Generated data in $DATA_DIR"
info "Partitions:"
ls -d "$DATA_DIR"/session_date=* 2>/dev/null | head -14 | while read -r d; do
    echo "  $(basename "$d")"
done

# ---------------------------------------------------------------
# Step 2: Simulate Spark aggregation for batch 1 (days 1-7)
# ---------------------------------------------------------------
step "Step 2: Simulate Spark int_visitor_daily (batch 1: 2024-07-01 to 2024-07-07)"

# Create the output directory structure
mkdir -p "$SPARK_WAREHOUSE"

# Use DuckDB CLI to aggregate raw sessions and write as Parquet.
# This simulates what the Spark model int_visitor_daily would produce.
# We partition the output by session_date using Hive partitioning.
duckdb -c "
COPY (
    SELECT
        visitor_id,
        CAST(session_date AS DATE) AS session_date,
        country,
        COUNT(*) AS session_count,
        SUM(page_views) AS total_page_views,
        COALESCE(SUM(revenue_cents), 0) AS total_revenue_cents,
        SUM(CASE WHEN is_converted THEN 1 ELSE 0 END) AS conversions,
        MIN(CAST(session_date AS TIMESTAMP)) AS first_session,
        MAX(CAST(session_date AS TIMESTAMP)) AS last_session,
        COUNT(DISTINCT traffic_source) AS traffic_source_count,
        COUNT(DISTINCT device_type) AS device_count
    FROM read_parquet('$DATA_DIR/**/*.parquet', hive_partitioning=true)
    WHERE session_date >= '2024-07-01' AND session_date < '2024-07-08'
    GROUP BY visitor_id, CAST(session_date AS DATE), country
) TO '$SPARK_WAREHOUSE' (FORMAT PARQUET, PARTITION_BY (session_date), OVERWRITE_OR_IGNORE);
"
info "Simulated Spark output for batch 1"
info "Partitions written:"
ls -d "$SPARK_WAREHOUSE"/session_date=* 2>/dev/null | while read -r d; do
    echo "  $(basename "$d")"
done

# ---------------------------------------------------------------
# Step 3: Run DuckDB mart models for batch 1
# ---------------------------------------------------------------
step "Step 3: Run DuckDB mart models (batch 1: 2024-07-01 to 2024-07-08)"

(cd "$SCRIPT_DIR" && $SMELT run \
    --project-dir . \
    --target duckdb_local \
    --select mart_daily_metrics --select mart_country_metrics \
    --start 2024-07-01 \
    --end 2024-07-08)

info "Batch 1 complete"

# ---------------------------------------------------------------
# Step 4: Query DuckDB to show batch 1 results
# ---------------------------------------------------------------
step "Step 4: Batch 1 results"

echo ""
echo "--- mart_daily_metrics (batch 1) ---"
duckdb "$DUCKDB_PATH" -c "
SELECT metric_date, unique_visitors, total_sessions, total_revenue, conversion_rate_pct
FROM analytics.mart_daily_metrics
ORDER BY metric_date;
"

echo ""
echo "--- mart_country_metrics (batch 1) ---"
duckdb "$DUCKDB_PATH" -c "
SELECT country, unique_visitors, total_sessions, total_revenue, conversion_rate_pct
FROM analytics.mart_country_metrics
ORDER BY total_revenue DESC
LIMIT 10;
"

# ---------------------------------------------------------------
# Step 5: Simulate Spark aggregation for batch 2 (days 8-14)
# ---------------------------------------------------------------
step "Step 5: Simulate Spark int_visitor_daily (batch 2: 2024-07-08 to 2024-07-14)"

duckdb -c "
COPY (
    SELECT
        visitor_id,
        CAST(session_date AS DATE) AS session_date,
        country,
        COUNT(*) AS session_count,
        SUM(page_views) AS total_page_views,
        COALESCE(SUM(revenue_cents), 0) AS total_revenue_cents,
        SUM(CASE WHEN is_converted THEN 1 ELSE 0 END) AS conversions,
        MIN(CAST(session_date AS TIMESTAMP)) AS first_session,
        MAX(CAST(session_date AS TIMESTAMP)) AS last_session,
        COUNT(DISTINCT traffic_source) AS traffic_source_count,
        COUNT(DISTINCT device_type) AS device_count
    FROM read_parquet('$DATA_DIR/**/*.parquet', hive_partitioning=true)
    WHERE session_date >= '2024-07-08' AND session_date < '2024-07-15'
    GROUP BY visitor_id, CAST(session_date AS DATE), country
) TO '$SPARK_WAREHOUSE' (FORMAT PARQUET, PARTITION_BY (session_date), OVERWRITE_OR_IGNORE);
"
info "Simulated Spark output for batch 2"
info "All partitions:"
ls -d "$SPARK_WAREHOUSE"/session_date=* 2>/dev/null | while read -r d; do
    echo "  $(basename "$d")"
done

# ---------------------------------------------------------------
# Step 6: Run DuckDB mart models for batch 2
# ---------------------------------------------------------------
step "Step 6: Run DuckDB mart models (batch 2: 2024-07-08 to 2024-07-15)"

(cd "$SCRIPT_DIR" && $SMELT run \
    --project-dir . \
    --target duckdb_local \
    --select mart_daily_metrics --select mart_country_metrics \
    --start 2024-07-08 \
    --end 2024-07-15)

info "Batch 2 complete"

# ---------------------------------------------------------------
# Step 7: Query DuckDB to show combined results
# ---------------------------------------------------------------
step "Step 7: Combined results (both batches)"

echo ""
echo "--- mart_daily_metrics (all 14 days) ---"
duckdb "$DUCKDB_PATH" -c "
SELECT metric_date, unique_visitors, total_sessions, total_revenue, conversion_rate_pct
FROM analytics.mart_daily_metrics
ORDER BY metric_date;
"

echo ""
echo "--- mart_country_metrics (full dataset) ---"
duckdb "$DUCKDB_PATH" -c "
SELECT country, unique_visitors, total_sessions, total_revenue, conversion_rate_pct
FROM analytics.mart_country_metrics
ORDER BY total_revenue DESC;
"

echo ""
echo "--- Summary ---"
duckdb "$DUCKDB_PATH" -c "
SELECT
    COUNT(*) AS total_days,
    MIN(metric_date) AS first_date,
    MAX(metric_date) AS last_date,
    SUM(total_sessions) AS grand_total_sessions,
    ROUND(SUM(total_revenue), 2) AS grand_total_revenue
FROM analytics.mart_daily_metrics;
"

step "Demo complete!"
echo ""
echo "The pipeline executed in two incremental batches:"
echo "  Batch 1: 2024-07-01 to 2024-07-07 (7 days)"
echo "  Batch 2: 2024-07-08 to 2024-07-14 (7 days)"
echo ""
echo "Cross-engine reference resolution:"
echo "  mart_daily_metrics   -> read_parquet(target/spark/.../int_visitor_daily/)"
echo "  mart_country_metrics -> read_parquet(target/spark/.../int_visitor_daily/)"
echo ""
echo "DuckDB database: $DUCKDB_PATH"
