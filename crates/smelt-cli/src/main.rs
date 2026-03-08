use anyhow::{Context, Result};
use arrow::util::pretty;
use chrono::{Duration, NaiveDate};
use clap::{Parser, Subcommand};
use smelt_backend::{Backend, PartitionSpec};
use smelt_backend_duckdb::DuckDbBackend;
use smelt_cli::{
    executor, find_project_root, inject_time_filter, parse_selector, BackendType, Config,
    DependencyGraph, ModelDiscovery, SourceConfig, SqlCompiler, TimeRange,
};
use smelt_db::{ColumnSource, Inputs, ModelSchema, TypeChecking};
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "spark")]
use smelt_backend_spark::SparkBackend;

#[derive(Parser)]
#[command(name = "smelt")]
#[command(about = "Modern data transformation framework", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run models and materialize them in the target database
    Run(RunArgs),
    /// Show column types for a model
    Table(TableArgs),
}

#[derive(Parser)]
struct RunArgs {
    /// Path to smelt project root
    #[arg(long, default_value = ".")]
    project_dir: PathBuf,

    /// DuckDB database file path
    #[arg(long)]
    database: Option<PathBuf>,

    /// Target environment from smelt.yml
    #[arg(long, default_value = "dev")]
    target: String,

    /// Display query results after execution
    #[arg(long)]
    show_results: bool,

    /// Show compiled SQL for each model
    #[arg(long, short)]
    verbose: bool,

    /// Parse and validate without executing
    #[arg(long)]
    dry_run: bool,

    /// Start of event time range for incremental models (ISO 8601: YYYY-MM-DD)
    #[arg(long = "event-time-start", requires = "event_time_end")]
    event_time_start: Option<String>,

    /// End of event time range for incremental models (exclusive, ISO 8601: YYYY-MM-DD)
    #[arg(long = "event-time-end", requires = "event_time_start")]
    event_time_end: Option<String>,

    /// Select models to run (repeatable). Supports: model_name, tag:X, +tag:X, tag:X+, +tag:X+
    #[arg(long = "select", short = 's')]
    select: Vec<String>,
}

#[derive(Parser)]
struct TableArgs {
    /// Name of the model to inspect
    model_name: String,

    /// Path to smelt project root
    #[arg(long, default_value = ".")]
    project_dir: PathBuf,

    /// Output format: table (default), json
    #[arg(long, default_value = "table")]
    format: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run(args) => run(args).await,
        Commands::Table(args) => table(args).await,
    }
}

async fn run(args: RunArgs) -> Result<()> {
    // 1. Find project root
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    println!("Project directory: {}", project_dir.display());

    // 2. Load configuration
    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;

    println!("Project: {} (version {})", config.name, config.version);

    // Get target config
    let target_config = config.targets.get(&args.target).ok_or_else(|| {
        anyhow::anyhow!(
            "Target '{}' not found in smelt.yml. Available targets: {}",
            args.target,
            config
                .targets
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;

    // Load source configuration (optional)
    let sources = SourceConfig::load(&project_dir).ok();

    if let Some(ref source_config) = sources {
        let source_count: usize = source_config.sources.values().map(|s| s.tables.len()).sum();
        println!("Loaded {} source tables", source_count);
    }

    // 3. Discover models
    let discovery = ModelDiscovery::new(project_dir.clone(), config.model_paths.clone());
    let models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;

    println!("Found {} models", models.len());

    // Report any parse errors
    for model in &models {
        if !model.parse_errors.is_empty() {
            eprintln!("\nWarning: Parse errors in {}:", model.name);
            for error in &model.parse_errors {
                eprintln!("  - {} at {:?}", error.message, error.range);
            }
        }
    }

    // 4. Build dependency graph
    let graph = DependencyGraph::build(models, sources.as_ref())
        .with_context(|| "Failed to build dependency graph")?;

    graph
        .validate()
        .with_context(|| "Dependency validation failed")?;

    // 5. Determine execution order (with optional selector filtering)
    let execution_order = if args.select.is_empty() {
        graph
            .execution_order()
            .with_context(|| "Failed to determine execution order")?
    } else {
        let selectors: Vec<_> = args
            .select
            .iter()
            .map(|s| parse_selector(s).with_context(|| format!("Invalid selector '{}'", s)))
            .collect::<Result<_, _>>()?;

        let selected = graph
            .select_models(&selectors, &config)
            .with_context(|| "Failed to select models")?;

        if selected.is_empty() {
            println!("No models matched the selectors");
            return Ok(());
        }

        graph
            .filtered_execution_order(&selected)
            .with_context(|| "Failed to determine execution order")?
    };

    println!(
        "\nExecution order: {}",
        execution_order
            .iter()
            .enumerate()
            .map(|(i, name)| format!("{}. {}", i + 1, name))
            .collect::<Vec<_>>()
            .join(" → ")
    );

    if args.dry_run {
        println!("\n[DRY RUN] Skipping execution");
        return Ok(());
    }

    // 6. Create backend based on target type
    let backend: Box<dyn Backend> = match target_config.backend_type() {
        BackendType::DuckDB => {
            let database = target_config
                .database
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("DuckDB target requires 'database' field"))?;

            let db_path = args.database.unwrap_or_else(|| project_dir.join(database));
            println!("\nBackend: DuckDB");
            println!("Database: {}", db_path.display());

            Box::new(
                DuckDbBackend::new(&db_path, &target_config.schema)
                    .await
                    .with_context(|| format!("Failed to initialize DuckDB at {:?}", db_path))?,
            )
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

                println!("\nBackend: Spark");
                println!("Connect URL: {}", connect_url);
                println!("Catalog: {}", catalog);

                Box::new(
                    SparkBackend::new(connect_url, catalog, &target_config.schema)
                        .await
                        .with_context(|| {
                            format!("Failed to connect to Spark at {}", connect_url)
                        })?,
                )
            }
            #[cfg(not(feature = "spark"))]
            {
                return Err(anyhow::anyhow!(
                    "Spark backend not available. Rebuild with --features spark"
                ));
            }
        }
    };

    // 7. Validate sources exist (if sources.yml present)
    if let Some(ref source_config) = sources {
        executor::validate_sources(backend.as_ref(), source_config)
            .await
            .with_context(|| "Source validation failed")?;
    }

    // 8. Parse time range if provided (for incremental processing)
    let time_range = match (&args.event_time_start, &args.event_time_end) {
        (Some(start), Some(end)) => {
            // Validate date format
            NaiveDate::parse_from_str(start, "%Y-%m-%d").with_context(|| {
                format!("Invalid start date format: {}. Expected YYYY-MM-DD", start)
            })?;
            NaiveDate::parse_from_str(end, "%Y-%m-%d").with_context(|| {
                format!("Invalid end date format: {}. Expected YYYY-MM-DD", end)
            })?;

            println!("\nTime range: {} to {} (exclusive)", start, end);
            Some(TimeRange {
                start: start.clone(),
                end: end.clone(),
            })
        }
        _ => None,
    };

    // 9. Compile and execute each model
    let compiler = SqlCompiler::new(config.clone());

    println!("\n{}", "=".repeat(60));
    println!("Executing models...");
    println!("{}", "=".repeat(60));

    let mut results = Vec::new();

    for model_name in &execution_order {
        let model = graph.get_model(model_name)?;

        // Check if this model should be run incrementally
        // SQL metadata takes precedence over smelt.yml
        let inc_config = config
            .get_incremental_with_metadata(model_name, model.metadata.as_ref().map(|b| b.as_ref()));
        let is_incremental = time_range.is_some() && inc_config.is_some();

        if is_incremental {
            let range = time_range.as_ref().unwrap();
            let inc = inc_config.unwrap();

            println!("\n▶ Running model: {} (incremental)", model_name);

            // Transform SQL to filter by time range
            let transformed_sql = inject_time_filter(&model.content, &inc.event_time_column, range)
                .with_context(|| format!("Failed to transform SQL for model: {}", model_name))?;

            // Compile with transformed SQL
            let compiled = compiler
                .compile_with_sql(model, &target_config.schema, &transformed_sql)
                .with_context(|| format!("Failed to compile model: {}", model_name))?;

            if args.verbose {
                println!("\n  Transformed SQL:");
                println!("  {}", "─".repeat(58));
                for line in compiled.sql.lines() {
                    println!("  {}", line);
                }
                println!("  {}", "─".repeat(58));
            }

            // Generate partition values for DELETE
            let partition_values = generate_partition_dates(&range.start, &range.end)?;
            println!(
                "  Partitions to update: {} ({} days)",
                if partition_values.len() <= 3 {
                    partition_values.join(", ")
                } else {
                    format!(
                        "{}, ..., {}",
                        partition_values.first().unwrap(),
                        partition_values.last().unwrap()
                    )
                },
                partition_values.len()
            );

            let partition = PartitionSpec {
                column: inc.partition_column.clone(),
                values: partition_values,
            };

            // Execute incrementally
            let result = executor::execute_model_incremental(
                backend.as_ref(),
                &compiled,
                &target_config.schema,
                partition,
                args.show_results,
            )
            .await
            .with_context(|| format!("Failed to execute model: {}", model_name))?;

            println!(
                "  ✓ {} ({} rows, {:?})",
                result.model_name, result.row_count, result.duration
            );

            // Show preview if requested
            if let Some(ref batches) = result.preview {
                println!("\n  Preview:");
                pretty::print_batches(batches).with_context(|| "Failed to print result preview")?;
                println!();
            }

            results.push(result);
        } else {
            // Standard full refresh path
            if time_range.is_some() && inc_config.is_none() {
                println!(
                    "\n▶ Running model: {} (full refresh - not configured for incremental)",
                    model_name
                );
            } else {
                println!("\n▶ Running model: {}", model_name);
            }

            // Compile
            let compiled = compiler
                .compile(model, &target_config.schema)
                .with_context(|| format!("Failed to compile model: {}", model_name))?;

            if args.verbose {
                println!("\n  Compiled SQL:");
                println!("  {}", "─".repeat(58));
                for line in compiled.sql.lines() {
                    println!("  {}", line);
                }
                println!("  {}", "─".repeat(58));
            }

            // Execute
            let result = executor::execute_model(
                backend.as_ref(),
                &compiled,
                &target_config.schema,
                args.show_results,
            )
            .await
            .with_context(|| format!("Failed to execute model: {}", model_name))?;

            println!(
                "  ✓ {} ({} rows, {:?})",
                result.model_name, result.row_count, result.duration
            );

            // Show preview if requested
            if let Some(ref batches) = result.preview {
                println!("\n  Preview:");
                pretty::print_batches(batches).with_context(|| "Failed to print result preview")?;
                println!();
            }

            results.push(result);
        }
    }

    // 9. Summary
    println!("\n{}", "=".repeat(60));
    println!("Summary");
    println!("{}", "=".repeat(60));
    println!("✓ Executed {} models successfully", results.len());

    let total_duration: std::time::Duration = results.iter().map(|r| r.duration).sum();
    println!("  Total time: {:?}", total_duration);

    Ok(())
}

/// Generate partition date values from a time range.
/// Returns a list of date strings in YYYY-MM-DD format.
fn generate_partition_dates(start: &str, end: &str) -> Result<Vec<String>> {
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

    let mut dates = Vec::new();
    let mut current = start_date;
    while current < end_date {
        dates.push(current.format("%Y-%m-%d").to_string());
        current += Duration::days(1);
    }

    Ok(dates)
}

async fn table(args: TableArgs) -> Result<()> {
    // 1. Find project root
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    // 2. Load configuration and discover models
    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;

    let discovery = ModelDiscovery::new(project_dir.clone(), config.model_paths.clone());
    let models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;

    // 3. Initialize Salsa database (same pattern as LSP)
    let mut db = smelt_db::Database::default();

    // Load sources.yml or sources.yaml
    let sources_yaml = {
        let yml_path = project_dir.join("sources.yml");
        let yaml_path = project_dir.join("sources.yaml");
        match (yml_path.exists(), yaml_path.exists()) {
            (true, true) => {
                eprintln!(
                    "Warning: Both sources.yml and sources.yaml exist in {}",
                    project_dir.display()
                );
                std::fs::read_to_string(&yml_path).unwrap_or_default()
            }
            (true, false) => std::fs::read_to_string(&yml_path).unwrap_or_default(),
            (false, true) => std::fs::read_to_string(&yaml_path).unwrap_or_default(),
            (false, false) => String::new(),
        }
    };
    db.set_project_sources_yaml(project_dir.clone(), Arc::new(sources_yaml));
    db.set_all_project_roots(Arc::new(vec![project_dir.clone()]));

    // Register all model files
    let mut file_paths = Vec::new();
    for model in &models {
        db.set_file_text(model.path.clone(), Arc::new(model.content.clone()));
        db.set_file_project_root(model.path.clone(), project_dir.clone());
        file_paths.push(model.path.clone());
    }
    db.set_all_files(Arc::new(file_paths));

    // 4. Find the model path
    let model = models
        .iter()
        .find(|m| m.name == args.model_name)
        .ok_or_else(|| anyhow::anyhow!("Model '{}' not found", args.model_name))?;

    // 5. Get typed schema
    let schema = db.typed_model_schema(model.path.clone());

    // 6. Output
    match args.format.as_str() {
        "json" => print_json(&schema, &args.model_name),
        _ => print_table(&schema, &args.model_name),
    }

    Ok(())
}

fn print_table(schema: &ModelSchema, model_name: &str) {
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

fn print_json(schema: &ModelSchema, model_name: &str) {
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

    println!("{}", to_string_pretty(&output).unwrap());
}
