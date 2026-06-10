use anyhow::{Context, Result};
use chrono::{Datelike, Duration, NaiveDate, Weekday as ChronoWeekday};
use smelt_backend::{Backend, IncrementalStrategy};
use smelt_core::{Granularity, Weekday};
use smelt_db::{ColumnSource, ModelSchema};
use std::path::{Path, PathBuf};

use tracing::info;

use smelt_cli::BackendType;

#[cfg(feature = "duckdb")]
use smelt_backend_duckdb::DuckDbBackend;

#[cfg(feature = "spark")]
use smelt_backend_spark::SparkBackend;

#[allow(dead_code)]
pub fn granularity_label(g: &Granularity) -> &'static str {
    match g {
        Granularity::Hour => "hours",
        Granularity::Day => "days",
        Granularity::Week => "weeks",
        Granularity::Month => "months",
        Granularity::Quarter => "quarters",
        Granularity::Year => "years",
    }
}

#[allow(dead_code)]
pub fn strategy_label(s: &IncrementalStrategy) -> &'static str {
    match s {
        IncrementalStrategy::DeleteInsert => "delete+insert",
        IncrementalStrategy::Append => "append",
        IncrementalStrategy::InsertOverwrite => "insert_overwrite",
    }
}

/// Infer deployed columns from the Salsa type inference database.
pub fn infer_deployed_columns(
    db: &smelt_db::Database,
    model: &smelt_cli::ModelFile,
) -> Vec<smelt_state::schema_tracking::DeployedColumn> {
    let ws = smelt_db::Workspace::try_get(db).expect("workspace not initialized");
    let file = db
        .source_file(&model.path)
        .expect("model file not registered");
    let schema = smelt_db::typed_model_schema(db, ws, file);

    schema
        .columns
        .iter()
        .filter(|c| c.name != "*")
        .map(|c| {
            let (data_type, nullable) = match &c.data_type {
                Some(tc) => (tc.data_type.to_sql(), tc.nullable),
                None => ("UNKNOWN".to_string(), true),
            };
            smelt_state::schema_tracking::DeployedColumn {
                name: c.name.clone(),
                data_type,
                nullable,
            }
        })
        .collect()
}

/// Generate partition values from a time range based on granularity.
///
/// `week_start` is the first day of the week for weekly partitions.
/// Only relevant when `granularity` is `Week`. When `None`, defaults to Monday.
#[allow(dead_code)]
pub fn generate_partition_values(
    start: &str,
    end: &str,
    granularity: &Granularity,
    week_start: Option<&Weekday>,
) -> Result<Vec<String>> {
    let start_date = NaiveDate::parse_from_str(start, "%Y-%m-%d")
        .with_context(|| format!("Invalid start date: {}", start))?;
    let end_date = NaiveDate::parse_from_str(end, "%Y-%m-%d")
        .with_context(|| format!("Invalid end date: {}", end))?;

    if start_date >= end_date {
        return Err(anyhow::anyhow!(
            "Start date ({}) must be before end date ({})",
            start,
            end
        ));
    }

    let mut values = Vec::new();

    match granularity {
        Granularity::Hour => {
            let mut current = start_date
                .and_hms_opt(0, 0, 0)
                .expect("midnight is always valid");
            let end_dt = end_date
                .and_hms_opt(0, 0, 0)
                .expect("midnight is always valid");
            while current < end_dt {
                values.push(current.format("%Y-%m-%d %H:00:00").to_string());
                current += Duration::hours(1);
            }
        }
        Granularity::Day => {
            let mut current = start_date;
            while current < end_date {
                values.push(current.format("%Y-%m-%d").to_string());
                current += Duration::days(1);
            }
        }
        Granularity::Week => {
            let default_weekday = Weekday::Monday;
            let ws = week_start.unwrap_or(&default_weekday);
            let chrono_day = weekday_to_chrono(ws);
            // Find first date >= start_date that falls on the week_start day
            let mut current = start_date;
            let days_ahead = (chrono_day.num_days_from_monday() as i64
                - current.weekday().num_days_from_monday() as i64
                + 7)
                % 7;
            if days_ahead > 0 {
                current += Duration::days(days_ahead);
            }
            while current < end_date {
                values.push(current.format("%Y-%m-%d").to_string());
                current += Duration::days(7);
            }
        }
        Granularity::Month => {
            let mut current = start_date;
            while current < end_date {
                values.push(current.format("%Y-%m").to_string());
                // Advance to next month
                let (y, m) = if current.month() == 12 {
                    (current.year() + 1, 1)
                } else {
                    (current.year(), current.month() + 1)
                };
                current = NaiveDate::from_ymd_opt(y, m, 1).expect("first of month is always valid");
            }
        }
        Granularity::Quarter => {
            let mut current = start_date;
            while current < end_date {
                let q = (current.month() - 1) / 3 + 1;
                values.push(format!("{}-Q{}", current.year(), q));
                // Advance to next quarter (3 months)
                let new_month = current.month() + 3;
                let (y, m) = if new_month > 12 {
                    (current.year() + 1, new_month - 12)
                } else {
                    (current.year(), new_month)
                };
                current = NaiveDate::from_ymd_opt(y, m, 1)
                    .expect("first of quarter month is always valid");
            }
        }
        Granularity::Year => {
            let mut current = start_date;
            while current < end_date {
                values.push(format!("{}", current.year()));
                current = NaiveDate::from_ymd_opt(current.year() + 1, 1, 1)
                    .expect("January 1st is always valid");
            }
        }
    }

    Ok(values)
}

/// Convert smelt `Weekday` to `chrono::Weekday`.
#[allow(dead_code)]
fn weekday_to_chrono(day: &Weekday) -> ChronoWeekday {
    match day {
        Weekday::Monday => ChronoWeekday::Mon,
        Weekday::Tuesday => ChronoWeekday::Tue,
        Weekday::Wednesday => ChronoWeekday::Wed,
        Weekday::Thursday => ChronoWeekday::Thu,
        Weekday::Friday => ChronoWeekday::Fri,
        Weekday::Saturday => ChronoWeekday::Sat,
        Weekday::Sunday => ChronoWeekday::Sun,
    }
}

#[allow(unreachable_code, unused_variables)]
pub async fn create_backend(
    target_config: &smelt_cli::config::Target,
    project_dir: &Path,
    database_override: Option<PathBuf>,
) -> Result<Box<dyn Backend>> {
    match target_config.backend_type() {
        BackendType::DuckDB => {
            #[cfg(feature = "duckdb")]
            {
                let database = target_config
                    .database
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("DuckDB target requires 'database' field"))?;

                let db_path = database_override.unwrap_or_else(|| project_dir.join(database));
                info!("Backend: DuckDB");
                info!("Database: {}", db_path.display());

                Ok(Box::new(
                    DuckDbBackend::new(&db_path, &target_config.schema)
                        .await
                        .with_context(|| format!("Failed to initialize DuckDB at {:?}", db_path))?,
                ))
            }
            #[cfg(not(feature = "duckdb"))]
            {
                Err(anyhow::anyhow!(
                    "DuckDB backend not available. Rebuild with --features duckdb"
                ))
            }
        }
        BackendType::Spark => {
            #[cfg(feature = "spark")]
            {
                let connect_url = target_config
                    .connect_url
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Spark target requires 'connect_url' field"))?;

                let default_catalog = "spark_catalog".to_string();
                let catalog = target_config.catalog.as_ref().unwrap_or(&default_catalog);

                info!("Backend: Spark");
                info!("Connect URL: {}", connect_url);
                info!("Catalog: {}", catalog);

                Ok(Box::new(
                    SparkBackend::new(
                        connect_url,
                        catalog,
                        &target_config.schema,
                        target_config.warehouse.as_deref(),
                    )
                    .await
                    .with_context(|| format!("Failed to connect to Spark at {}", connect_url))?,
                ))
            }
            #[cfg(not(feature = "spark"))]
            {
                Err(anyhow::anyhow!(
                    "Spark backend not available. Rebuild with --features spark"
                ))
            }
        }
    }
}

pub fn print_table(schema: &ModelSchema, model_name: &str) {
    println!("Model: {}\n", model_name);
    println!("{:<30} {:<20} Nullable", "Column", "Type");
    println!("{}", "-".repeat(60));

    for col in &schema.columns {
        // Skip wildcards
        if col.name == "*" {
            continue;
        }

        let (type_str, nullable) = match &col.data_type {
            Some(t) => (
                t.data_type.to_string(),
                if t.nullable { "yes" } else { "no" },
            ),
            None => ("UNKNOWN".to_string(), "?"),
        };

        println!("{:<30} {:<20} {}", col.name, type_str, nullable);
    }
}

pub fn print_json(schema: &ModelSchema, model_name: &str) {
    use serde_json::{json, to_string_pretty};

    let columns: Vec<_> = schema
        .columns
        .iter()
        .filter(|col| col.name != "*")
        .map(|col| {
            let (data_type, nullable) = match &col.data_type {
                Some(t) => (t.data_type.to_string(), t.nullable),
                None => ("UNKNOWN".to_string(), true),
            };

            let source = match &col.source {
                ColumnSource::FromModel {
                    model_name,
                    column_name,
                } => json!({
                    "type": "from_model",
                    "model": model_name,
                    "column": column_name
                }),
                ColumnSource::Computed => json!({
                    "type": "computed",
                    "expression": col.expression
                }),
                ColumnSource::Wildcard { model_name } => json!({
                    "type": "wildcard",
                    "model": model_name
                }),
                ColumnSource::ExternalTable { table_name } => json!({
                    "type": "external_table",
                    "table": table_name
                }),
                ColumnSource::Unknown => json!({ "type": "unknown" }),
            };

            json!({
                "name": col.name,
                "data_type": data_type,
                "nullable": nullable,
                "expression": col.expression,
                "source": source
            })
        })
        .collect();

    let output = json!({
        "model": model_name,
        "columns": columns
    });

    println!(
        "{}",
        to_string_pretty(&output).expect("JSON serialization of schema should not fail")
    );
}

pub fn print_property_test_result(
    result: &smelt_cli::test_property::PropertyTestResult,
    verbose: bool,
    show_all: bool,
) {
    let cte_suffix = result
        .target_cte
        .as_ref()
        .map(|c| format!("::{}", c))
        .unwrap_or_default();
    let iter_tag = format!("[{} iter]", result.iterations);
    let name_width = result.name.len() + result.model.len() + cte_suffix.len() + iter_tag.len();

    if result.passed {
        if show_all {
            println!(
                "  PASS {} ({}{}) {}{:>width$}",
                result.name,
                result.model,
                cte_suffix,
                iter_tag,
                format!("{:.2}s", result.duration.as_secs_f64()),
                width = 40usize.saturating_sub(name_width),
            );
        }
    } else {
        println!(
            "  FAIL {} ({}{}) {}{:>width$}",
            result.name,
            result.model,
            cte_suffix,
            iter_tag,
            format!("{:.2}s", result.duration.as_secs_f64()),
            width = 40usize.saturating_sub(name_width),
        );
        if let Some(ref inner) = result.inner_result {
            if let Some(ref error) = inner.error {
                println!("\n{}", error);
                if verbose && !inner.compiled_sql.is_empty() {
                    println!("  Compiled SQL:");
                    println!("    {}", inner.compiled_sql.replace('\n', "\n    "));
                }
            }
            println!();
        }
    }
}

pub fn print_test_result(
    result: &smelt_cli::test_runner::TestResult,
    verbose: bool,
    show_all: bool,
) {
    let cte_suffix = result
        .target_cte
        .as_ref()
        .map(|c| format!("::{}", c))
        .unwrap_or_default();

    if result.passed {
        if show_all {
            println!(
                "  PASS {} ({}{}){:>width$}",
                result.name,
                result.model,
                cte_suffix,
                format!("{:.2}s", result.duration.as_secs_f64()),
                width = 40usize
                    .saturating_sub(result.name.len() + result.model.len() + cte_suffix.len())
            );
        }
    } else {
        println!(
            "  FAIL {} ({}{}){:>width$}",
            result.name,
            result.model,
            cte_suffix,
            format!("{:.2}s", result.duration.as_secs_f64()),
            width =
                40usize.saturating_sub(result.name.len() + result.model.len() + cte_suffix.len())
        );
        if let Some(ref error) = result.error {
            println!("\n{}", error);
            if verbose && !result.compiled_sql.is_empty() {
                println!("  Compiled SQL:");
                println!("    {}", result.compiled_sql.replace('\n', "\n    "));
            }
            println!();
        }
    }
}
