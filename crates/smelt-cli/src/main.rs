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

    /// Log line format: human-readable text, or one parseable JSON object
    /// per line (for orchestrator/log-aggregator consumption).
    #[arg(long, global = true, value_enum, default_value_t = LogFormat::Text)]
    log_format: LogFormat,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum LogFormat {
    Text,
    Json,
}

#[derive(Subcommand)]
enum Commands {
    /// Non-interactively scaffold a minimal working project
    Init(InitArgs),
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
    /// Measure per-cell maintenance technique cost over replayed windows of real data
    Bakeoff(BakeoffArgs),
    /// Show pending schema changes between model definitions and deployed state
    Diff(DiffArgs),
    /// Print the definition-delta migration plan for a changed model (plan-only; executes nothing)
    Migrate(MigrateArgs),
    /// Run unit tests for models
    Test(TestArgs),
    /// Run data-quality checks against the configured target
    Check(CheckArgs),
    /// List discovered project entities (models, seeds, sources, tests, checks)
    List(ListArgs),
    /// Remove build artifacts under target/ (never touches state)
    Clean(CleanArgs),
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
struct InitArgs {
    /// Target directory to scaffold (created if it doesn't exist).
    /// Defaults to the current directory. Refused (exit 2) if it already
    /// contains a smelt.yml — there is no --force to override this.
    dir: Option<PathBuf>,
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

    /// Print the resolved execution plan (model names + strategies) and exit.
    /// Combine with --dry-run to see the plan without executing.
    #[arg(long = "show-plan")]
    show_plan: bool,

    /// Forward propagation: run exactly the partitions dirtied by the
    /// caller-declared per-source deltas (`--source`/`--landed`), computed
    /// through the maintenance-plan propagation graph
    /// (`incremental_models.md` §"The graph layer"). Requires at least one
    /// `--source`.
    #[arg(long = "since-upstream")]
    since_upstream: bool,

    /// A source address whose landed delta is declared via the paired
    /// `--landed` flag (repeatable — the Nth `--source` pairs with the Nth
    /// `--landed` positionally, or `--landed <address>=<start>..<end>` pairs
    /// by address). A `--source` with no paired `--landed` resolves from its
    /// persisted watermark (`run_state.md` §"Per-source watermark"). Only
    /// meaningful with `--since-upstream`.
    #[arg(long = "source", requires = "since_upstream")]
    since_upstream_source: Vec<String>,

    /// The landed interval for the paired `--source`: bare `<start>..<end>`
    /// (positional pairing) or `<address>=<start>..<end>` (pairing by
    /// address) — ISO `YYYY-MM-DD`, end exclusive. Repeatable; see
    /// `--source`.
    #[arg(long = "landed", requires = "since_upstream")]
    since_upstream_landed: Vec<String>,

    /// Maximum number of models to execute concurrently. Defaults to the
    /// host's available parallelism. `--jobs 1` forces strictly serial
    /// execution, one model at a time — a dependency edge always keeps the
    /// upstream model's completion before its downstream's start
    /// regardless of this value.
    #[arg(long = "jobs", short = 'j')]
    jobs: Option<usize>,

    /// Resume a previously partially-failed run: skip any model that
    /// succeeded last time with an unchanged definition, rerun everything
    /// else (plus its downstream dependents). Errors if the most recent run
    /// completed successfully or no run manifest exists — there is nothing
    /// to resume from (`docs/specs/run_state.md` §"`--resume` semantics").
    #[arg(long = "resume")]
    resume: bool,
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

    /// Allow binding to a non-loopback host. The UI server has no
    /// authentication, so binding it to an address reachable from other
    /// machines requires this explicit opt-in.
    #[arg(long = "allow-remote")]
    allow_remote: bool,
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
    /// Optional path to a single model file to plan (with --show-plan), or a
    /// model name/selector to backward-resolve (with --include-upstreams).
    /// Ignored otherwise.
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

    /// Backward resolution: given the target model (the positional
    /// argument) and this period, resolve the per-ancestor required
    /// upstream slices and the ancestor-first/target-last build order
    /// through the same propagation graph `--since-upstream` assembles
    /// (`incremental_models.md` §"Backward resolution — what must exist"),
    /// print them, and build exactly that bounded set. Requires
    /// `--include-upstreams`.
    #[arg(long = "period", requires = "include_upstreams")]
    period: Option<String>,

    /// Resolve and build the target model's required upstream slices for
    /// `--period` (backward resolution) instead of the ordinary
    /// seed+run-everything build. Requires `--period` and the target model
    /// as the positional argument.
    #[arg(long = "include-upstreams", requires = "period")]
    include_upstreams: bool,
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

    /// Target environment from smelt.yml — state is partitioned per target,
    /// so status is reported for this target's state only.
    #[arg(long, default_value = "dev")]
    target: String,

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

    /// Target environment from smelt.yml — state is partitioned per target,
    /// so history is reported for this target's runs only.
    #[arg(long, default_value = "dev")]
    target: String,

    /// Specific model to show history for (omit for all runs)
    model_name: Option<String>,

    /// Number of runs to show (default: 10)
    #[arg(long, short, default_value = "10")]
    limit: usize,
}

#[derive(Parser)]
struct ExplainArgs {
    /// Name of a single model to print the maintenance plan report for
    /// (cells, clamps, locality verdicts, inbound edges). Omit to print the
    /// whole-project dependency graph as before.
    model_name: Option<String>,

    /// Path to smelt project root
    #[arg(long, default_value = ".")]
    project_dir: PathBuf,

    /// Output as JSON (required for machine consumption)
    #[arg(long)]
    json: bool,

    /// Select models to include (repeatable). Supports: model_name, tag:X, +tag:X, tag:X+, +tag:X+
    #[arg(long = "select", short = 's')]
    select: Vec<String>,

    /// Print, after each cell's report block, the maintenance statements
    /// that cell executes — the output of the same pure emitters a run
    /// executes (`docs/specs/incremental_models.md` §"Statement emission
    /// (single owner)"). Only meaningful with a positional model-name
    /// argument. Never connects to a backend.
    #[arg(long = "show-sql")]
    show_sql: bool,

    /// Region literal bounds for `--show-sql`, `<start>..<end>`
    /// (`YYYY-MM-DD`, end exclusive). Without this flag, the printed
    /// statements use the symbolic placeholders `{{window_start}}`/
    /// `{{window_end}}` instead of real literals.
    #[arg(long = "period", requires = "show_sql")]
    period: Option<String>,

    /// Render a named technique's own preview statements instead of the
    /// admitted one's, for every cell — including a cell where this
    /// technique is structurally `NotApplicable`, whose reason is reported
    /// rather than silently omitted (`docs/specs/ui_model_diagnostics.md`
    /// §Surface "CLI"). Requires `--show-sql`. Accepts: `delete_insert`,
    /// `keyed_fold`, `column_scoped_merge`, `in_place_update`, `recompute`
    /// (`recompute` and `delete_insert` both resolve to the same
    /// DELETE+INSERT / region-recompute technique — there is no separate
    /// recompute emitter). Never affects `--json`, which always carries
    /// every technique's preview regardless of this flag.
    #[arg(long = "technique", requires = "show_sql")]
    technique: Option<String>,
}

#[derive(Parser)]
struct BakeoffArgs {
    /// Model to measure
    model_name: String,

    /// Path to smelt project root
    #[arg(long, default_value = ".")]
    project_dir: PathBuf,

    /// Target environment from smelt.yml to clone for scratch measurement runs.
    #[arg(long, default_value = "dev")]
    target: String,

    /// Narrow measurement to specific cells: `<col>@<source>`, repeatable
    /// and/or comma-separated. Defaults to every cell with 2+ admissible
    /// techniques.
    #[arg(long = "cells")]
    cells: Vec<String>,

    /// Number of sequential replayed windows to slice the driving source's
    /// event-time extent into.
    #[arg(long, default_value = "3")]
    runs: u32,

    /// Retain the scratch schemas (and their per-target state dirs) after
    /// measurement instead of dropping them.
    #[arg(long)]
    keep: bool,

    /// Print the winning technique per measured cell as ready-to-paste
    /// `cells[]` YAML. Emit-only — never writes the model's `.sql` file.
    /// Not yet implemented (`docs/plans/20260719-prod-w7-bakeoff.md` Phase 5).
    #[arg(long)]
    pin: bool,
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

    /// Target environment from smelt.yml — deployed schemas are recorded
    /// per target, so the diff compares against this target's state.
    #[arg(long, default_value = "dev")]
    target: String,

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
struct MigrateArgs {
    /// Name of the model to derive a migration plan for
    model: String,

    /// Path to smelt project root
    #[arg(long, default_value = ".")]
    project_dir: PathBuf,

    /// Target environment from smelt.yml — the deployed-schema snapshot the
    /// diff's "before" side is read from is recorded per target.
    #[arg(long, default_value = "dev")]
    target: String,

    /// DuckDB database file path override — only opened by `--apply` on a
    /// matching hash, to execute the approved plan's statements.
    #[arg(long)]
    database: Option<PathBuf>,

    /// Execute the most recently approved plan — refuses (exit 3) if the
    /// freshly re-derived plan hash doesn't match the recorded one, or if
    /// the approved plan itself refuses to execute (a skeleton-change
    /// group, a group with no admissible candidate, or a destructive
    /// candidate).
    #[arg(long)]
    apply: bool,

    /// Emit the plan as machine-readable JSON instead of the human-readable
    /// report (CI mode; `docs/specs/definition_deltas.md` §Surface "`smelt
    /// migrate`").
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

    /// Output results as JSON for editor integration (exits 0 regardless of test status)
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct CheckArgs {
    /// Path to smelt project root
    #[arg(long, default_value = ".")]
    project_dir: PathBuf,

    /// Select specific checks to run by name substring (repeatable)
    #[arg(long = "select", short = 's')]
    select: Vec<String>,

    /// Target environment from smelt.yml
    #[arg(long, default_value = "dev")]
    target: String,

    /// DuckDB database file path (overrides smelt.yml)
    #[arg(long)]
    database: Option<PathBuf>,

    /// Show compiled SQL for each check
    #[arg(long, short)]
    verbose: bool,
}

#[derive(Parser)]
struct ListArgs {
    /// Path to smelt project root
    #[arg(long, default_value = ".")]
    project_dir: PathBuf,

    /// Select entities to list (repeatable). Supports: model_name, tag:X, +tag:X, tag:X+, +tag:X+
    #[arg(long = "select", short = 's')]
    select: Vec<String>,

    /// Exclude entities from the listing (repeatable). Same syntax as --select.
    #[arg(long = "exclude", short = 'e')]
    exclude: Vec<String>,

    /// Output format: "text" (default) or "json"
    #[arg(long, default_value = "text")]
    format: String,
}

#[derive(Parser)]
struct CleanArgs {
    /// Path to smelt project root
    #[arg(long, default_value = ".")]
    project_dir: PathBuf,
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = Cli::parse();

    match cli.log_format {
        LogFormat::Text => {
            tracing_subscriber::fmt()
                .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
                .init();
        }
        LogFormat::Json => {
            tracing_subscriber::fmt()
                .json()
                .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
                .init();
        }
    }

    let scope = cli.scope.as_deref();

    // `smelt init` classifies its own error (the non-empty-dir refusal maps
    // to exit 2, a usage error per `docs/specs/cli.md`) via
    // `commands::init::exit_code_for` rather than the generic
    // `smelt_cli::exit_code_for`, since its error type isn't one of the
    // `ProjectError`/`ConfigError` variants that classifier recognizes.
    let is_init = matches!(cli.command, Commands::Init(_));
    // `smelt list` maps parse errors and unresolvable/ambiguous selectors to
    // exit 2 (usage error) per `docs/specs/cli.md` §"Exit codes" — a distinct
    // classifier from the generic one, same pattern as `init` above.
    let is_list = matches!(cli.command, Commands::List(_));

    let result: Result<()> = match cli.command {
        Commands::Init(args) => commands::init::run(args),
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
        Commands::Bakeoff(args) => commands::bakeoff::bakeoff(args, scope).await,
        Commands::Diff(args) => commands::diff::diff(args, scope).await,
        Commands::Migrate(args) => commands::migrate::migrate(args, scope).await,
        Commands::Test(args) => commands::test::run_tests(args).await,
        Commands::Check(args) => commands::check::run_checks(args).await,
        Commands::List(args) => commands::list::list(args, scope).await,
        Commands::Clean(args) => commands::clean::clean(args).await,
        Commands::Docs { command } => match command {
            DocsCommands::Generate(args) => commands::docs::generate(args).await,
            DocsCommands::List => commands::docs::list(),
            DocsCommands::Show { topic } => commands::docs::show(&topic),
            DocsCommands::Path => commands::docs::path(),
        },
    };

    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {err:?}");
            let code = if is_init {
                commands::init::exit_code_for(&err)
            } else if is_list {
                commands::list::exit_code_for(&err)
            } else {
                smelt_cli::exit_code_for(&err)
            };
            std::process::ExitCode::from(code)
        }
    }
}
