use anyhow::{Context, Result};
use chrono::Utc;
use smelt_cli::find_project_root;
use smelt_state::file_store::FileStore;

use crate::StatusArgs;

pub async fn status(args: StatusArgs) -> Result<()> {
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    let file_store = FileStore::new(&project_dir);
    if !file_store.exists() {
        println!("No state directory found. Run `smelt run` with a time range first.");
        return Ok(());
    }

    let interval_store = file_store
        .load_intervals()
        .with_context(|| "Failed to load interval store")?;

    if interval_store.models.is_empty() {
        println!("No interval data recorded yet.");
        return Ok(());
    }

    let today = Utc::now().format("%Y-%m-%d").to_string();
    let until = args.until.as_deref().unwrap_or(&today);

    let models_to_show: Vec<(&String, &smelt_state::intervals::ModelIntervals)> =
        if let Some(ref name) = args.model_name {
            interval_store
                .get(name)
                .map(|i| vec![(name, i)])
                .unwrap_or_else(|| {
                    tracing::warn!("Model '{}' not found in interval store", name);
                    vec![]
                })
        } else {
            let mut v: Vec<_> = interval_store.models.iter().collect();
            v.sort_by_key(|(k, _)| (*k).clone());
            v
        };

    println!("Interval Coverage Status");
    println!("{}", "=".repeat(60));

    for (model_name, intervals) in &models_to_show {
        println!("\n  {}", model_name);
        println!("  {}", "-".repeat(40));

        if intervals.covered_intervals.is_empty() {
            println!("    No coverage (model hash changed or never run)");
            continue;
        }

        for interval in &intervals.covered_intervals {
            println!("    Covered: {} to {}", interval.start, interval.end);
        }

        if let Some(since) = args.since.as_deref().or(intervals
            .earliest_date()
            .as_ref()
            .map(|_| intervals.covered_intervals[0].start.as_str()))
        {
            let gaps = intervals.find_gaps(since, until);
            if gaps.is_empty() {
                println!("    No gaps in [{}, {})", since, until);
            } else {
                for gap in &gaps {
                    println!("    GAP: {} to {}", gap.start, gap.end);
                }
            }
        }

        println!("    Hash: {}", intervals.model_hash);
    }

    Ok(())
}
