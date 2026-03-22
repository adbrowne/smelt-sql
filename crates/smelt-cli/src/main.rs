use anyhow::{Context, Result};
use arrow::util::pretty;
use chrono::{Datelike, Duration, NaiveDate, Weekday as ChronoWeekday};
use clap::{Parser, Subcommand};
use smelt_backend::{Backend, IncrementalStrategy, PartitionSpec};
#[cfg(feature = "duckdb")]
use smelt_backend_duckdb::DuckDbBackend;
use smelt_cli::{
    discover_python_models, executor, find_project_root, init_db, inject_time_filter,
    parse_selector, resolve_refs_in_sql, seed, BackendType, Config, DependencyGraph,
    ModelDiscovery, SourcesConfig, SqlCompiler, TimeRange,
};
use smelt_core::{Granularity, IncrementalConfig, Weekday};
use smelt_db::{ColumnSource, ModelSchema, TypeChecking};
use smelt_optimizer::{
    ExecutionStep, Frontmatter, ModelGraph, ModelInfo, Optimizer, Transformation,
};
use std::path::{Path, PathBuf};

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
    /// Start the web UI for visualizing the model graph
    Ui(UiArgs),
    /// Load seed CSV files into the database
    Seed(SeedArgs),
    /// Seed the database then run all models (seed + run)
    Build(BuildArgs),
    /// Show the function type signature of models (inputs -> outputs)
    Type(TypeArgs),
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
struct UiArgs {
    /// Path to smelt project root
    #[arg(long, default_value = ".")]
    project_dir: PathBuf,

    /// Port to serve the UI on
    #[arg(long, default_value = "3000")]
    port: u16,

    /// Host address to bind to
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
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

#[derive(Parser)]
struct SeedArgs {
    /// Path to smelt project root
    #[arg(long, default_value = ".")]
    project_dir: PathBuf,

    /// DuckDB database file path
    #[arg(long)]
    database: Option<PathBuf>,

    /// Target environment from smelt.yml
    #[arg(long, default_value = "dev")]
    target: String,

    /// Display loaded data after seeding
    #[arg(long)]
    show_results: bool,

    /// Select specific seeds to load (by name or schema.name)
    #[arg(long = "select", short = 's')]
    select: Vec<String>,
}

#[derive(Parser)]
struct BuildArgs {
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
struct TypeArgs {
    /// Name of the model to inspect (omit to show all models)
    model_name: Option<String>,

    /// Path to smelt project root
    #[arg(long, default_value = ".")]
    project_dir: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run(args) => run(args).await,
        Commands::Table(args) => table(args).await,
        Commands::Ui(args) => ui(args).await,
        Commands::Seed(args) => run_seed(args).await,
        Commands::Build(args) => build(args).await,
        Commands::Type(args) => show_type(args).await,
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
    let sources = SourcesConfig::load(&project_dir).ok();

    if let Some(ref source_config) = sources {
        let source_count: usize = source_config.sources.iter().map(|s| s.tables.len()).sum();
        println!("Loaded {} source tables", source_count);
    }

    // 3. Discover models (SQL + Python)
    let discovery = ModelDiscovery::new(project_dir.clone(), config.model_paths.clone());
    let mut models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;

    // Discover and execute Python models
    let python_files = discovery
        .discover_python_files()
        .with_context(|| "Failed to scan for Python models")?;

    if !python_files.is_empty() {
        let python_models = discover_python_models(
            &python_files,
            &models,
            &config,
            &project_dir,
            config.python.as_deref(),
        )
        .with_context(|| "Failed to discover Python models")?;

        if !python_models.is_empty() {
            println!(
                "Found {} Python model(s) from {} file(s)",
                python_models.len(),
                python_files.len()
            );
            models.extend(python_models);
        }
    }

    if models.is_empty() {
        return Err(anyhow::anyhow!(
            "No models found in model paths: {}",
            config.model_paths.join(", ")
        ));
    }

    println!("Found {} models total", models.len());

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
    let backend = create_backend(target_config, &project_dir, args.database).await?;

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

    // 9. Run optimizer on discovered models
    let mut opt_graph = ModelGraph::new();
    for model_name in &execution_order {
        let model = graph.get_model(model_name)?;
        let frontmatter = Frontmatter::parse(&model.content);
        opt_graph.add_model(ModelInfo {
            name: model.name.clone(),
            sql: model.content.clone(),
            refs: model.refs.iter().map(|r| r.model_name.clone()).collect(),
            incremental_config: frontmatter.as_ref().and_then(|f| f.incremental.clone()),
        });
    }

    let optimizer = Optimizer::new();
    let (transformations, opt_errors) = optimizer.optimize(&opt_graph);

    for err in &opt_errors {
        eprintln!("  Optimizer error: {}", err);
    }

    // Build lookup maps for transformations
    let mut plan_overrides: std::collections::HashMap<String, Vec<smelt_optimizer::ExecutionStep>> =
        std::collections::HashMap::new();
    let mut incremental_overrides: std::collections::HashMap<
        String,
        (String, String, Granularity),
    > = std::collections::HashMap::new(); // model -> (event_time_column, partition_column, granularity)

    for t in &transformations {
        match t {
            Transformation::ReplaceWithPlan { model, steps } => {
                plan_overrides.insert(model.clone(), steps.clone());
            }
            Transformation::SetIncremental {
                model,
                event_time_column,
                partition_column,
                granularity,
            } => {
                incremental_overrides.insert(
                    model.clone(),
                    (
                        event_time_column.clone(),
                        partition_column.clone(),
                        granularity.clone(),
                    ),
                );
            }
        }
    }

    if !plan_overrides.is_empty() || !incremental_overrides.is_empty() {
        println!("\nOptimizer:");
        for model in plan_overrides.keys() {
            println!("  {} → cube split", model);
        }
        for (model, (_, partition_col, _)) in &incremental_overrides {
            println!("  {} → incremental (partition: {})", model, partition_col);
        }
    }

    // Mandatory time range check: if any model has SetIncremental and no time range provided
    if !incremental_overrides.is_empty() && time_range.is_none() {
        let inc_models: Vec<&String> = incremental_overrides.keys().collect();
        return Err(anyhow::anyhow!(
            "Incremental models selected for run ({}) but --event-time-start and --event-time-end not provided.\n\
             These are required when running incremental models, similar to dbt's microbatch parameters.",
            inc_models
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    // 10. Compile and execute each model
    let compiler = SqlCompiler::new(config.clone(), target_config);

    println!("\n{}", "=".repeat(60));
    println!("Executing models...");
    println!("{}", "=".repeat(60));

    let mut results = Vec::new();

    for model_name in &execution_order {
        let model = graph.get_model(model_name)?;

        // Determine execution mode from optimizer overrides or config
        let plan_steps = plan_overrides.get(model_name.as_str()).map(|steps| {
            steps
                .iter()
                .map(|step| match step {
                    ExecutionStep::CreateTemp { name, sql } => ExecutionStep::CreateTemp {
                        name: name.clone(),
                        sql: resolve_refs_in_sql(sql, &target_config.schema),
                    },
                    ExecutionStep::AppendToTemp { name, sql } => ExecutionStep::AppendToTemp {
                        name: name.clone(),
                        sql: resolve_refs_in_sql(sql, &target_config.schema),
                    },
                    ExecutionStep::FinalQuery { sql } => {
                        ExecutionStep::FinalQuery { sql: sql.clone() }
                    }
                    ExecutionStep::DropTemp { name } => {
                        ExecutionStep::DropTemp { name: name.clone() }
                    }
                })
                .collect::<Vec<_>>()
        });

        let execution = if let Some((event_time_col, partition_col, granularity)) =
            incremental_overrides.get(model_name.as_str())
        {
            // Optimizer detected incremental
            ModelExecution::Incremental {
                config: IncrementalConfig {
                    enabled: true,
                    event_time_column: event_time_col.clone(),
                    partition_column: partition_col.clone(),
                    granularity: granularity.clone(),
                    unique_key: vec![],
                    safety_overrides: Default::default(),
                },
                time_range: time_range.as_ref().unwrap().clone(),
                plan_steps,
            }
        } else if let Some(inc) = config
            .get_incremental_with_metadata(model_name, model.metadata.as_ref().map(|b| b.as_ref()))
            .filter(|_| time_range.is_some())
        {
            // Config-based incremental (smelt.yml or frontmatter)
            ModelExecution::Incremental {
                config: inc.clone(),
                time_range: time_range.as_ref().unwrap().clone(),
                plan_steps,
            }
        } else if let Some(steps) = plan_steps {
            // Cube split only (full refresh with plan)
            ModelExecution::CubeSplit { steps }
        } else {
            ModelExecution::FullRefresh
        };

        let result = match &execution {
            ModelExecution::Incremental {
                config: inc_config,
                time_range: range,
                plan_steps: Some(steps),
            } => {
                let resolved_strategy = backend.resolve_strategy(inc_config);
                println!(
                    "\n▶ Running model: {} (cube split + incremental/{})",
                    model_name,
                    strategy_label(&resolved_strategy),
                );

                let partition_values =
                    generate_partition_values(&range.start, &range.end, &inc_config.granularity)?;
                let partition = PartitionSpec {
                    column: inc_config.partition_column.clone(),
                    values: partition_values,
                };

                executor::execute_plan_incremental(
                    backend.as_ref(),
                    model_name,
                    steps,
                    &target_config.schema,
                    partition,
                    &inc_config.event_time_column,
                    range,
                    resolved_strategy,
                    inc_config.unique_key.clone(),
                    args.show_results,
                )
                .await
                .with_context(|| format!("Failed to execute model: {}", model_name))?
            }
            ModelExecution::CubeSplit { steps } => {
                // Cube split only (full refresh)
                println!("\n▶ Running model: {} (cube split)", model_name);

                executor::execute_plan(
                    backend.as_ref(),
                    model_name,
                    steps,
                    &target_config.schema,
                    args.show_results,
                )
                .await
                .with_context(|| format!("Failed to execute model: {}", model_name))?
            }
            ModelExecution::Incremental {
                config: inc_config,
                time_range: range,
                plan_steps: None,
            } => {
                let resolved_strategy = backend.resolve_strategy(inc_config);
                println!(
                    "\n▶ Running model: {} (incremental/{})",
                    model_name,
                    strategy_label(&resolved_strategy),
                );

                let clean_sql = smelt_parser::strip_frontmatter(&model.content);
                let transformed_sql =
                    inject_time_filter(&clean_sql, &inc_config.event_time_column, range)
                        .with_context(|| {
                            format!("Failed to transform SQL for model: {}", model_name)
                        })?;

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

                let partition_values =
                    generate_partition_values(&range.start, &range.end, &inc_config.granularity)?;
                println!(
                    "  Partitions to update: {} ({} {})",
                    if partition_values.len() <= 3 {
                        partition_values.join(", ")
                    } else {
                        format!(
                            "{}, ..., {}",
                            partition_values.first().unwrap(),
                            partition_values.last().unwrap()
                        )
                    },
                    partition_values.len(),
                    granularity_label(&inc_config.granularity),
                );

                let partition = PartitionSpec {
                    column: inc_config.partition_column.clone(),
                    values: partition_values,
                };

                executor::execute_model_incremental(
                    backend.as_ref(),
                    &compiled,
                    &target_config.schema,
                    partition,
                    resolved_strategy,
                    inc_config.unique_key.clone(),
                    args.show_results,
                )
                .await
                .with_context(|| format!("Failed to execute model: {}", model_name))?
            }
            ModelExecution::FullRefresh => {
                if time_range.is_some() {
                    println!(
                        "\n▶ Running model: {} (full refresh - not configured for incremental)",
                        model_name
                    );
                } else {
                    println!("\n▶ Running model: {}", model_name);
                }

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

                executor::execute_model(
                    backend.as_ref(),
                    &compiled,
                    &target_config.schema,
                    args.show_results,
                )
                .await
                .with_context(|| format!("Failed to execute model: {}", model_name))?
            }
        };

        println!(
            "  ✓ {} ({} rows, {:?})",
            result.model_name, result.row_count, result.duration
        );

        if let Some(ref batches) = result.preview {
            println!("\n  Preview:");
            pretty::print_batches(batches).with_context(|| "Failed to print result preview")?;
            println!();
        }

        results.push(result);
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

/// Describes how a model should be executed.
enum ModelExecution {
    FullRefresh,
    CubeSplit {
        steps: Vec<ExecutionStep>,
    },
    Incremental {
        config: IncrementalConfig,
        time_range: TimeRange,
        plan_steps: Option<Vec<ExecutionStep>>,
    },
}

fn granularity_label(g: &Granularity) -> &'static str {
    match g {
        Granularity::Hour => "hours",
        Granularity::Day => "days",
        Granularity::Week { .. } => "weeks",
        Granularity::Month => "months",
        Granularity::Quarter => "quarters",
        Granularity::Year => "years",
    }
}

fn strategy_label(s: &IncrementalStrategy) -> &'static str {
    match s {
        IncrementalStrategy::DeleteInsert => "delete+insert",
        IncrementalStrategy::Merge => "merge",
        IncrementalStrategy::Append => "append",
        IncrementalStrategy::InsertOverwrite => "insert_overwrite",
    }
}

/// Generate partition values from a time range based on granularity.
fn generate_partition_values(
    start: &str,
    end: &str,
    granularity: &Granularity,
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
            let mut current = start_date.and_hms_opt(0, 0, 0).unwrap();
            let end_dt = end_date.and_hms_opt(0, 0, 0).unwrap();
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
        Granularity::Week { week_start } => {
            let chrono_day = weekday_to_chrono(week_start);
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
                current = NaiveDate::from_ymd_opt(y, m, 1).unwrap();
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
                current = NaiveDate::from_ymd_opt(y, m, 1).unwrap();
            }
        }
        Granularity::Year => {
            let mut current = start_date;
            while current < end_date {
                values.push(format!("{}", current.year()));
                current = NaiveDate::from_ymd_opt(current.year() + 1, 1, 1).unwrap();
            }
        }
    }

    Ok(values)
}

/// Convert smelt `Weekday` to `chrono::Weekday`.
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
async fn create_backend(
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
                println!("\nBackend: DuckDB");
                println!("Database: {}", db_path.display());

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

                println!("\nBackend: Spark");
                println!("Connect URL: {}", connect_url);
                println!("Catalog: {}", catalog);

                Ok(Box::new(
                    SparkBackend::new(connect_url, catalog, &target_config.schema)
                        .await
                        .with_context(|| {
                            format!("Failed to connect to Spark at {}", connect_url)
                        })?,
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

async fn run_seed(args: SeedArgs) -> Result<()> {
    // 1. Find project root and load config
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    println!("Project directory: {}", project_dir.display());

    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;

    println!("Project: {} (version {})", config.name, config.version);

    // 2. Get target config
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

    // 3. Discover seeds
    let mut seeds = seed::discover_seeds(&project_dir, &config.seed_paths, &target_config.schema)
        .with_context(|| "Failed to discover seeds")?;

    if seeds.is_empty() {
        println!("No seed files found in: {}", config.seed_paths.join(", "));
        return Ok(());
    }

    // 4. Filter by --select if provided
    if !args.select.is_empty() {
        seeds = seed::filter_seeds(seeds, &args.select);
        if seeds.is_empty() {
            println!("No seeds matched selectors: {}", args.select.join(", "));
            return Ok(());
        }
    }

    println!("Found {} seed(s)", seeds.len());

    // 5. Create backend
    let backend = create_backend(target_config, &project_dir, args.database).await?;

    // 6. Execute seeds
    println!("\n{}", "=".repeat(60));
    println!("Seeding...");
    println!("{}", "=".repeat(60));

    let mut results = Vec::new();

    for s in &seeds {
        let type_label = match s.seed_type {
            seed::SeedType::Source => "source",
            seed::SeedType::Target => "target",
        };
        println!("\n▶ Seeding: {} ({})", s.qualified_name(), type_label);

        let result = seed::execute_seed(backend.as_ref(), s, args.show_results)
            .await
            .with_context(|| format!("Failed to seed '{}'", s.qualified_name()))?;

        println!(
            "  ✓ {} ({} rows, {:?})",
            result.qualified_name, result.row_count, result.duration
        );

        results.push(result);
    }

    // 7. Summary
    println!("\n{}", "=".repeat(60));
    println!("Summary");
    println!("{}", "=".repeat(60));
    println!("✓ Loaded {} seed(s) successfully", results.len());

    let total_rows: usize = results.iter().map(|r| r.row_count).sum();
    let total_duration: std::time::Duration = results.iter().map(|r| r.duration).sum();
    println!("  Total rows: {}", total_rows);
    println!("  Total time: {:?}", total_duration);

    Ok(())
}

async fn build(args: BuildArgs) -> Result<()> {
    // Step 1: Seed
    let seed_args = SeedArgs {
        project_dir: args.project_dir.clone(),
        database: args.database.clone(),
        target: args.target.clone(),
        show_results: false,
        select: Vec::new(),
    };
    run_seed(seed_args).await?;

    // Step 2: Run
    let run_args = RunArgs {
        project_dir: args.project_dir,
        database: args.database,
        target: args.target,
        show_results: args.show_results,
        verbose: args.verbose,
        dry_run: false,
        event_time_start: args.event_time_start,
        event_time_end: args.event_time_end,
        select: args.select,
    };
    run(run_args).await
}

async fn table(args: TableArgs) -> Result<()> {
    // 1. Find project root
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    // 2. Load configuration and discover models
    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;

    let discovery = ModelDiscovery::new(project_dir.clone(), config.model_paths.clone());
    let mut models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;

    // Discover Python models for table command too
    let python_files = discovery
        .discover_python_files()
        .with_context(|| "Failed to scan for Python models")?;

    if !python_files.is_empty() {
        let python_models = discover_python_models(
            &python_files,
            &models,
            &config,
            &project_dir,
            config.python.as_deref(),
        )
        .with_context(|| "Failed to discover Python models")?;
        models.extend(python_models);
    }

    // 3. Initialize Salsa database
    let db = init_db(&project_dir, &models);

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

async fn ui(args: UiArgs) -> Result<()> {
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;

    let sources = SourcesConfig::load(&project_dir).ok();

    let discovery = ModelDiscovery::new(project_dir.clone(), config.model_paths.clone());
    let models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;

    println!("Found {} models", models.len());

    // Initialize smelt-db for schema queries
    let db = init_db(&project_dir, &models);

    // Build a smelt-core DependencyGraph for UI builders (they use smelt-core types)
    let core_models: Vec<smelt_core::ModelFile> = models
        .iter()
        .map(|m| smelt_core::ModelFile {
            name: m.name.clone(),
            path: m.path.clone(),
            content: m.content.clone(),
            refs: m.refs.clone(),
            parse_errors: m.parse_errors.clone(),
            metadata: m.metadata.clone(),
        })
        .collect();
    let core_graph = smelt_core::graph::DependencyGraph::build(core_models, sources.as_ref())
        .with_context(|| "Failed to build UI dependency graph")?;

    // Build responses using smelt-ui builders
    let project_response =
        smelt_ui::build::build_project_response(&config, &core_graph, sources.as_ref());
    let graph_response = smelt_ui::build::build_graph_response(&core_graph, &config);
    let model_details = smelt_ui::build::build_model_details(&core_graph, &config, &db);

    smelt_ui::start_server(
        project_response,
        graph_response,
        model_details,
        args.port,
        &args.host,
    )
    .await
}

async fn show_type(args: TypeArgs) -> Result<()> {
    // 1. Find project root
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    // 2. Load configuration and discover models
    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;

    let discovery = ModelDiscovery::new(project_dir.clone(), config.model_paths.clone());
    let mut models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;

    // Discover Python models
    let python_files = discovery
        .discover_python_files()
        .with_context(|| "Failed to scan for Python models")?;

    if !python_files.is_empty() {
        let python_models = discover_python_models(
            &python_files,
            &models,
            &config,
            &project_dir,
            config.python.as_deref(),
        )
        .with_context(|| "Failed to discover Python models")?;
        models.extend(python_models);
    }

    // 3. Initialize Salsa database
    let db = init_db(&project_dir, &models);

    // 4. Show function types
    if let Some(model_name) = &args.model_name {
        // Show single model
        let model = models
            .iter()
            .find(|m| m.name == *model_name)
            .ok_or_else(|| anyhow::anyhow!("Model '{}' not found", model_name))?;
        let ft = db.model_function_type(model.path.clone());
        println!("{}", ft);
    } else {
        // Show all models sorted by name
        let mut model_list: Vec<_> = models.iter().collect();
        model_list.sort_by(|a, b| a.name.cmp(&b.name));

        for (i, model) in model_list.iter().enumerate() {
            if i > 0 {
                println!();
            }
            let ft = db.model_function_type(model.path.clone());
            println!("{}", ft);
        }
    }

    Ok(())
}
