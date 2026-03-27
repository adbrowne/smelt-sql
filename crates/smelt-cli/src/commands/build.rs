use anyhow::Result;

use crate::{BuildArgs, RunArgs, SeedArgs};

pub async fn build(args: BuildArgs) -> Result<()> {
    // Step 1: Seed
    let seed_args = SeedArgs {
        project_dir: args.project_dir.clone(),
        database: args.database.clone(),
        target: args.target.clone(),
        show_results: false,
        select: Vec::new(),
    };
    super::seed::run_seed(seed_args).await?;

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
        exclude: args.exclude,
        start: None,
        end: None,
        batch_size: None,
        per_partition: false,
        auto: false,
        allow_column_removal: false,
    };
    super::run::run(run_args).await
}
