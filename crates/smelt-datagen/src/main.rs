//! CLI for deterministic data generation.

use anyhow::Result;
use chrono::NaiveDate;
use clap::Parser;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

#[derive(Parser, Debug)]
#[command(name = "smelt-datagen")]
#[command(about = "Deterministic data generation for smelt")]
struct Args {
    /// Path to YAML config file (replaces all other flags when provided)
    #[arg(long)]
    config: Option<PathBuf>,

    /// Output directory for Hive-partitioned Parquet files
    #[arg(short, long, default_value = "output")]
    output: PathBuf,

    /// Random seed for deterministic generation
    #[arg(short, long, default_value = "42")]
    seed: u64,

    /// Number of sessions to generate
    #[arg(short, long, default_value = "100000000")]
    num_sessions: usize,

    /// Number of days to spread sessions across
    #[arg(short, long, default_value = "30")]
    days: u32,

    /// Start date (YYYY-MM-DD)
    #[arg(long, default_value = "2024-01-01")]
    start_date: String,

    /// Scale factor (multiplies each dataset's num_rows; overrides config value)
    #[arg(long)]
    scale_factor: Option<f64>,

    /// Quiet mode (no progress output)
    #[arg(short, long)]
    quiet: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    if let Some(config_path) = args.config {
        let text = std::fs::read_to_string(&config_path)
            .map_err(|e| anyhow::anyhow!("Failed to read config {:?}: {}", config_path, e))?;
        let config: smelt_datagen::config::DatagenConfig = serde_yaml::from_str(&text)
            .map_err(|e| anyhow::anyhow!("Failed to parse config: {}", e))?;
        run_config(config, args.scale_factor, args.quiet)
    } else {
        run_session_generator(args)
    }
}

fn run_config(
    config: smelt_datagen::config::DatagenConfig,
    cli_scale_factor: Option<f64>,
    quiet: bool,
) -> Result<()> {
    use smelt_datagen::config::{FkCounts, GeneratorSpec};

    let global_seed = config.seed.unwrap_or(42);
    let scale_factor = cli_scale_factor.or(config.scale_factor).unwrap_or(1.0);

    if !quiet {
        println!("Scale factor: {}", scale_factor);
    }

    // Build FK resolution map: dataset name -> scaled row count
    // Also validate that FK references only refer to previously-listed datasets
    let mut fk_counts = FkCounts::new();
    for dataset in &config.datasets {
        // Validate FK references
        for col in &dataset.columns {
            if let GeneratorSpec::ForeignKey {
                dataset: ref target,
            } = col.generator
            {
                if !fk_counts.contains_key(target) {
                    anyhow::bail!(
                        "Dataset '{}' column '{}' references dataset '{}' via foreign_key, \
                         but '{}' has not been listed yet. \
                         Move it before '{}' in the config.",
                        dataset.name,
                        col.name,
                        target,
                        target,
                        dataset.name,
                    );
                }
            }
        }
        let scaled_rows = ((dataset.num_rows as f64) * scale_factor).round() as usize;
        fk_counts.insert(dataset.name.clone(), scaled_rows);
    }

    for dataset in &config.datasets {
        let scaled_rows = ((dataset.num_rows as f64) * scale_factor).round() as usize;
        // Create a scaled copy of the dataset config
        let mut scaled_dataset = dataset.clone();
        scaled_dataset.num_rows = scaled_rows;

        if !quiet {
            println!(
                "Generating dataset '{}' ({} rows) -> {}",
                scaled_dataset.name, scaled_dataset.num_rows, scaled_dataset.output
            );
        }

        let start_time = Instant::now();
        let last_print = AtomicU64::new(0);
        let total_rows = scaled_dataset.num_rows;

        let progress_fn = |current: usize, total: usize| {
            let elapsed = start_time.elapsed().as_secs();
            let last = last_print.load(Ordering::Relaxed);
            if elapsed > last {
                last_print.store(elapsed, Ordering::Relaxed);
                let pct = (current as f64 / total as f64) * 100.0;
                let rate = current as f64 / elapsed.max(1) as f64;
                let eta = if rate > 0.0 && current < total {
                    ((total - current) as f64 / rate) as u64
                } else {
                    0
                };
                eprint!(
                    "\r  {:.1}% ({}/{}) - {:.0} rows/sec - ETA: {}s    ",
                    pct, current, total, rate, eta
                );
            }
        };

        let progress: Option<&(dyn Fn(usize, usize) + Sync)> =
            if quiet { None } else { Some(&progress_fn) };

        let count = smelt_datagen::generic_parquet::write_generic_dataset(
            &scaled_dataset,
            global_seed,
            progress,
            &fk_counts,
        )?;

        let elapsed = start_time.elapsed();

        if !quiet {
            eprintln!();
            println!(
                "  Done: {} rows in {:.2}s ({:.0} rows/sec)",
                count,
                elapsed.as_secs_f64(),
                count as f64 / elapsed.as_secs_f64(),
            );
            let _ = total_rows;
        }
    }

    Ok(())
}

fn run_session_generator(args: Args) -> Result<()> {
    let start_date = NaiveDate::parse_from_str(&args.start_date, "%Y-%m-%d")
        .map_err(|e| anyhow::anyhow!("Invalid date format: {}", e))?;

    if !args.quiet {
        println!(
            "Generating {} sessions over {} days",
            args.num_sessions, args.days
        );
        println!("Output: {:?}", args.output);
        println!("Seed: {}", args.seed);
        println!();
    }

    let start_time = Instant::now();
    let last_print = AtomicU64::new(0);

    let progress_fn = |current: usize, total: usize| {
        let elapsed = start_time.elapsed().as_secs();
        let last = last_print.load(Ordering::Relaxed);

        // Print at most every second
        if elapsed > last {
            last_print.store(elapsed, Ordering::Relaxed);
            let pct = (current as f64 / total as f64) * 100.0;
            let rate = current as f64 / elapsed.max(1) as f64;
            let eta = if rate > 0.0 && current < total {
                ((total - current) as f64 / rate) as u64
            } else {
                0
            };
            eprint!(
                "\rProgress: {:.1}% ({}/{}) - {:.0} rows/sec - ETA: {}s    ",
                pct, current, total, rate, eta
            );
        }
    };

    let progress: Option<&(dyn Fn(usize, usize) + Sync)> =
        if args.quiet { None } else { Some(&progress_fn) };

    let count = smelt_datagen::parquet::write_sessions_to_parquet(
        &args.output,
        args.seed,
        args.num_sessions,
        args.days,
        start_date,
        progress,
    )?;

    let elapsed = start_time.elapsed();

    if !args.quiet {
        eprintln!();
        println!();
        println!(
            "Generated {} sessions in {:.2}s",
            count,
            elapsed.as_secs_f64()
        );
        println!("Rate: {:.0} rows/sec", count as f64 / elapsed.as_secs_f64());
    }

    Ok(())
}
