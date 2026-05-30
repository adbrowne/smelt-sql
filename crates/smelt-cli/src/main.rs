mod commands;
mod helpers;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// Validate a --scope value at parse time.
fn parse_scope(s: &str) -> Result<String, String> {
    smelt_cli::argument_resolution::validate_scope_value(s)?;
    Ok(s.to_string())
}

#[derive(Parser)]
#[command(name = "smelt")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "Modern data transformation framework", long_about = None)]
struct Cli {
    /// Scope prefix for argument resolution (dot-separated path, e.g. "silver").
    /// Overrides cwd-derived scope. Pass "" to disable auto-scope.
    #[arg(long, global = true, value_parser = parse_scope)]
    scope: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run models and materialize them in the target database
    Run(RunArgs),
    /// Backbuild: rebuild a target model and all its upstreams for a time range
    Backbuild(BackbuildArgs),
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
    /// Show interval coverage and gaps for incremental models
    Status(StatusArgs),
    /// Show run history
    History(HistoryArgs),
    /// Output model graph and configuration as JSON for orchestrator integration
    Explain(ExplainArgs),
    /// Show pending schema changes between model definitions and deployed state
    Diff(DiffArgs),
    /// Run unit tests for models
    Test(TestArgs),
    /// Generate documentation
    Docs {
        #[command(subcommand)]
        command: DocsCommands,
    },
}

#[derive(Subcommand)]
enum DocsCommands {
    /// Generate a data catalog / data dictionary
    Generate(DocsGenerateArgs),
    /// List user-facing documentation topics shipped with this binary
    List,
    /// Print the markdown contents of a documentation topic to stdout
    Show {
        /// Topic path, e.g. "getting-started/quickstart" (with or without .md)
        topic: String,
    },
    /// Explain where the embedded docs live
    Path,
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

    /// Exclude models from the run (repeatable). Same syntax as --select.
    #[arg(long = "exclude", short = 'e')]
    exclude: Vec<String>,

    /// Start of range for backfill (ISO 8601: YYYY-MM-DD). Alias for --event-time-start.
    #[arg(long)]
    start: Option<String>,

    /// End of range for backfill (exclusive, ISO 8601: YYYY-MM-DD). Alias for --event-time-end.
    #[arg(long)]
    end: Option<String>,

    /// Override batch size in days for backfill chunking
    #[arg(long = "batch-size")]
    batch_size: Option<u32>,

    /// Force per-partition execution (one query per granularity period)
    #[arg(long = "per-partition")]
    per_partition: bool,

    /// Auto mode: process only uncovered intervals since last run
    #[arg(long)]
    auto: bool,

    /// Allow column removal during schema evolution (otherwise blocked for safety)
    #[arg(long = "allow-column-removal")]
    allow_column_removal: bool,

    /// Allow full table refresh when schema evolution requires it (e.g., unsupported type change on Spark+Parquet)
    #[arg(long = "allow-full-refresh")]
    allow_full_refresh: bool,

    /// Allow incremental models that fail the safety classifier to fall back to
    /// full-table refresh instead of being refused at planning time.
    /// Use this only as a temporary escape hatch while fixing the model SQL.
    #[arg(long = "allow-downgrade")]
    allow_downgrade: bool,
}

#[derive(Parser)]
struct BackbuildArgs {
    /// Target model selector (e.g., +daily_revenue, model_name)
    selector: String,

    /// Start of time range (ISO 8601: YYYY-MM-DD)
    #[arg(long)]
    start: String,

    /// End of time range (exclusive, ISO 8601: YYYY-MM-DD)
    #[arg(long)]
    end: String,

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

    /// Show what would execute without running
    #[arg(long)]
    dry_run: bool,

    /// Override batch size in days for backfill chunking
    #[arg(long = "batch-size")]
    batch_size: Option<u32>,

    /// Force per-partition execution (one query per granularity period)
    #[arg(long = "per-partition")]
    per_partition: bool,

    /// Allow incremental models that fail bound derivation to fall back to
    /// full-table refresh instead of being refused at planning time.
    /// Use this only as a temporary escape hatch while fixing the model SQL.
    #[arg(long = "allow-downgrade")]
    allow_downgrade: bool,
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
    /// Optional path to a single model file to plan. Required with --show-plan,
    /// ignored otherwise.
    file: Option<PathBuf>,

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

    /// Exclude models from the run (repeatable). Same syntax as --select.
    #[arg(long = "exclude", short = 'e')]
    exclude: Vec<String>,

    /// Print the optimised logical plan for the given file and exit. Requires
    /// a model file as a positional argument. No execution side effects.
    #[arg(long = "show-plan")]
    show_plan: bool,

    /// Allow incremental models that fail the safety classifier to fall back to
    /// full-table refresh instead of being refused at planning time.
    /// Use this only as a temporary escape hatch while fixing the model SQL.
    #[arg(long = "allow-downgrade")]
    allow_downgrade: bool,
}

#[derive(Parser)]
struct TypeArgs {
    /// Name of the model to inspect (omit to show all models)
    model_name: Option<String>,

    /// Path to smelt project root
    #[arg(long, default_value = ".")]
    project_dir: PathBuf,
}

#[derive(Parser)]
struct StatusArgs {
    /// Path to smelt project root
    #[arg(long, default_value = ".")]
    project_dir: PathBuf,

    /// Specific model to show status for (omit for all)
    model_name: Option<String>,

    /// Start of query range for gap detection (ISO 8601: YYYY-MM-DD)
    #[arg(long)]
    since: Option<String>,

    /// End of query range for gap detection (ISO 8601: YYYY-MM-DD, default: today)
    #[arg(long)]
    until: Option<String>,
}

#[derive(Parser)]
struct HistoryArgs {
    /// Path to smelt project root
    #[arg(long, default_value = ".")]
    project_dir: PathBuf,

    /// Specific model to show history for (omit for all runs)
    model_name: Option<String>,

    /// Number of runs to show (default: 10)
    #[arg(long, short, default_value = "10")]
    limit: usize,
}

#[derive(Parser)]
struct ExplainArgs {
    /// Path to smelt project root
    #[arg(long, default_value = ".")]
    project_dir: PathBuf,

    /// Output as JSON (required for machine consumption)
    #[arg(long)]
    json: bool,

    /// Select models to include (repeatable). Supports: model_name, tag:X, +tag:X, tag:X+, +tag:X+
    #[arg(long = "select", short = 's')]
    select: Vec<String>,
}

#[derive(Parser)]
pub struct DocsGenerateArgs {
    /// Path to smelt project root
    #[arg(long, default_value = ".")]
    project_dir: PathBuf,

    /// Output format: markdown, json
    #[arg(long, default_value = "markdown")]
    format: String,

    /// Output directory (default: <project>/target/docs)
    #[arg(long, short)]
    output: Option<PathBuf>,

    /// Select models to include (repeatable). Supports: model_name, tag:X, +tag:X, tag:X+, +tag:X+
    #[arg(long = "select", short = 's')]
    select: Vec<String>,
}

#[derive(Parser)]
struct DiffArgs {
    /// Path to smelt project root
    #[arg(long, default_value = ".")]
    project_dir: PathBuf,

    /// Select models to diff (repeatable). Supports: model_name, tag:X, +tag:X, tag:X+, +tag:X+
    #[arg(long = "select", short = 's')]
    select: Vec<String>,

    /// Exclude models from diff (repeatable). Same syntax as --select.
    #[arg(long = "exclude", short = 'e')]
    exclude: Vec<String>,

    /// Output as JSON
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct TestArgs {
    /// Path to smelt project root
    #[arg(long, default_value = ".")]
    project_dir: PathBuf,

    /// Select specific tests to run (repeatable)
    #[arg(long = "select", short = 's')]
    select: Vec<String>,

    /// Show compiled SQL for each test
    #[arg(long, short)]
    verbose: bool,

    /// Show passing tests too (default: only failures)
    #[arg(long)]
    show_all: bool,

    /// Target environment from smelt.yml (for singular tests that query real data)
    #[arg(long, default_value = "dev")]
    target: String,

    /// DuckDB database file path (overrides smelt.yml)
    #[arg(long)]
    database: Option<PathBuf>,

    /// Random seed for property-based tests (for reproducibility)
    #[arg(long)]
    seed: Option<u64>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let scope = cli.scope.as_deref();

    match cli.command {
        Commands::Run(args) => commands::run::run(args, scope).await,
        Commands::Backbuild(args) => commands::backbuild::backbuild(args, scope).await,
        Commands::Table(args) => commands::table::table(args, scope).await,
        Commands::Ui(args) => commands::ui::ui(args).await,
        Commands::Seed(args) => commands::seed::run_seed(args, scope).await,
        Commands::Build(args) => commands::build::build(args, scope).await,
        Commands::Type(args) => commands::r#type::show_type(args, scope).await,
        Commands::Status(args) => commands::status::status(args, scope).await,
        Commands::History(args) => commands::history::history(args, scope).await,
        Commands::Explain(args) => commands::explain::explain(args, scope).await,
        Commands::Diff(args) => commands::diff::diff(args, scope).await,
        Commands::Test(args) => commands::test::run_tests(args).await,
        Commands::Docs { command } => match command {
            DocsCommands::Generate(args) => commands::docs::generate(args).await,
            DocsCommands::List => commands::docs::list(),
            DocsCommands::Show { topic } => commands::docs::show(&topic),
            DocsCommands::Path => commands::docs::path(),
        },
    }
}
