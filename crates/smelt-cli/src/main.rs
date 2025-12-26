use anyhow::{Context, Result};
use arrow::util::pretty;
use clap::{Parser, Subcommand};
use smelt_cli::{
    executor, find_project_root, Config, DependencyGraph, ModelDiscovery, SourceConfig,
    SqlCompiler,
};
use smelt_backend_duckdb::DuckDbBackend;
use std::path::PathBuf;

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
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run(args) => run(args).await,
    }
}

async fn run(args: RunArgs) -> Result<()> {
    // 1. Find project root
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    println!("Project directory: {}", project_dir.display());

    // 2. Load configuration
    let config = Config::load(&project_dir)
        .with_context(|| "Failed to load smelt.yml configuration")?;

    println!("Project: {} (version {})", config.name, config.version);

    // Get target config and extract values we need
    let (target_database, target_schema) = {
        let target_config = config.targets.get(&args.target).ok_or_else(|| {
            anyhow::anyhow!(
                "Target '{}' not found in smelt.yml. Available targets: {}",
                args.target,
                config.targets.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        })?;
        (target_config.database.clone(), target_config.schema.clone())
    };

    // Load source configuration (optional)
    let sources = SourceConfig::load(&project_dir).ok();

    if let Some(ref source_config) = sources {
        let source_count: usize = source_config
            .sources
            .values()
            .map(|s| s.tables.len())
            .sum();
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

    // 5. Determine execution order
    let execution_order = graph
        .execution_order()
        .with_context(|| "Failed to determine execution order")?;

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

    // 6. Setup DuckDB
    let db_path = args.database.unwrap_or_else(|| {
        project_dir.join(&target_database)
    });

    println!("\nDatabase: {}", db_path.display());

    let backend = DuckDbBackend::new(&db_path, &target_schema)
        .await
        .with_context(|| format!("Failed to initialize DuckDB at {:?}", db_path))?;

    // 7. Validate sources exist (if sources.yml present)
    if let Some(ref source_config) = sources {
        executor::validate_sources(&backend, source_config)
            .await
            .with_context(|| "Source validation failed")?;
    }

    // 8. Compile and execute each model
    let compiler = SqlCompiler::new(config);

    println!("\n{}", "=".repeat(60));
    println!("Executing models...");
    println!("{}", "=".repeat(60));

    let mut results = Vec::new();

    for model_name in &execution_order {
        let model = graph.get_model(model_name)?;

        println!("\n▶ Running model: {}", model_name);

        // Compile
        let compiled = compiler
            .compile(model, &target_schema)
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
        let result = executor::execute_model(&backend, &compiled, &target_schema, args.show_results)
            .await
            .with_context(|| format!("Failed to execute model: {}", model_name))?;

        println!(
            "  ✓ {} ({} rows, {:?})",
            result.model_name, result.row_count, result.duration
        );

        // Show preview if requested
        if let Some(ref batches) = result.preview {
            println!("\n  Preview:");
            pretty::print_batches(batches)
                .with_context(|| "Failed to print result preview")?;
            println!();
        }

        results.push(result);
    }

    // 9. Summary
    println!("\n{}", "=".repeat(60));
    println!("Summary");
    println!("{}", "=".repeat(60));
    println!("✓ Executed {} models successfully", results.len());
    println!("  Database: {}", db_path.display());

    let total_duration: std::time::Duration = results.iter().map(|r| r.duration).sum();
    println!("  Total time: {:?}", total_duration);

    Ok(())
}
