use anyhow::{anyhow, Context, Result};

use crate::{BuildArgs, RunArgs, SeedArgs};

pub async fn build(args: BuildArgs, scope: Option<&str>) -> Result<()> {
    if args.show_plan {
        return show_plan(args);
    }

    // Step 1: Seed
    let seed_args = SeedArgs {
        project_dir: args.project_dir.clone(),
        database: args.database.clone(),
        target: args.target.clone(),
        show_results: false,
        select: Vec::new(),
    };
    super::seed::run_seed(seed_args, scope).await?;

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
        allow_full_refresh: false,
        allow_downgrade: args.allow_downgrade,
        show_plan: false,
        since_upstream: false,
        since_upstream_source: Vec::new(),
        since_upstream_landed: Vec::new(),
    };
    super::run::run(run_args, scope).await
}

fn show_plan(args: BuildArgs) -> Result<()> {
    use std::path::Path;

    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_core::find_project_root_by_walking_up;
    use smelt_db::Workspace;
    use smelt_planner::logical_plan_rules::{apply_rules_to_fixed_point, show_plan_rules};
    use smelt_planner::plan_printer::format_plan;

    let file = args.file.ok_or_else(|| {
        anyhow!("--show-plan requires a model file path as a positional argument")
    })?;

    let abs_file = std::fs::canonicalize(&file)
        .with_context(|| format!("Failed to resolve model file path: {}", file.display()))?;

    // Prefer an explicit --project-dir if the user passed one (i.e. anything
    // other than the default "."); otherwise walk up from the file.
    let project_dir = if args.project_dir != Path::new(".") {
        args.project_dir.clone()
    } else {
        find_project_root_by_walking_up(&abs_file).ok_or_else(|| {
            anyhow!(
                "Could not locate smelt.yml above {}; pass --project-dir explicitly",
                abs_file.display()
            )
        })?
    };

    let config = Config::load(&project_dir).with_context(|| "Failed to load smelt.yml")?;

    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let mut models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;
    let function_files = discovery
        .discover_function_files()
        .with_context(|| "Failed to discover function files")?;
    models.extend(function_files);

    let db = init_db(&project_dir, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let source_file = db.source_file(&abs_file).ok_or_else(|| {
        anyhow!(
            "File {} is not registered in the workspace",
            abs_file.display()
        )
    })?;

    // Path-prefix enforcement: same rule as the build path. Reject
    // stem-included call paths before printing the plan.
    {
        let diags = smelt_db::file_diagnostics(&db, ws, source_file);
        let mut fn_path_errors: Vec<String> = Vec::new();
        for diag in diags {
            if diag.code == Some(smelt_db::DiagnosticCode::UnknownSmeltFn) {
                fn_path_errors.push(diag.message.clone());
            }
        }
        if !fn_path_errors.is_empty() {
            for err in &fn_path_errors {
                eprintln!("error: {err}");
            }
            return Err(anyhow!(
                "Unknown smelt function call(s) — the filename stem is not a path component; \
                 see `smelt docs show concepts/functions`.\n{}",
                fn_path_errors.join("\n")
            ));
        }
    }

    let plan = smelt_db::logical_plan(&db, ws, source_file)
        .ok_or_else(|| anyhow!("File {} did not parse as a valid model", abs_file.display()))?;

    let optimised = apply_rules_to_fixed_point(plan, &show_plan_rules());

    print!("{}", format_plan(&optimised));
    Ok(())
}
