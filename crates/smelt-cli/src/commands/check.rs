//! `smelt check` — run data-quality checks against the configured target.
//!
//! Each `smelt.check` is a failing-rows SELECT: zero rows = PASS, one or more
//! rows = violation. `error`-severity violations cause a nonzero exit code;
//! `warn`-severity violations are reported but do not affect the exit code.
//!
//! A check whose referenced model has not been built (i.e. the relation is absent
//! from the target schema) fails with a `CheckTargetNotBuilt` message and exit 1.
//! This is never a silent pass — the fail-loud discipline applies.
//!
//! Compilation path (run-pipeline parity rule):
//!   The check body SELECT is compiled through `CompilerRegistry::get(target)
//!   .compile_with_sql_and_ephemerals(check_model, schema, body_sql, resolver)`.
//!   No private compiler constructor is used. The EphemeralResolver is built the
//!   same way as in `execute_project`: ephemeral models in dependency order from
//!   the graph, via `compiler.build_ephemeral_resolver(models, schema)`.

use anyhow::{Context, Result};
use smelt_cli::{
    backend_registry::BackendRegistry, build_fn_body_map_from_model_files, find_project_root,
    CompilerRegistry, Config, ModelDiscovery,
};
use smelt_core::{graph::DependencyGraph, metadata::CheckSeverity};
use std::collections::{HashMap, HashSet};

use crate::CheckArgs;

// ── Public entry point ──────────────────────────────────────────────────────

pub async fn run_checks(args: CheckArgs) -> Result<()> {
    #[cfg(not(feature = "duckdb"))]
    {
        return Err(anyhow::anyhow!(
            "The 'check' command requires the 'duckdb' feature. \
             Build with: cargo build --features duckdb"
        ));
    }

    #[cfg(feature = "duckdb")]
    run_checks_inner(args).await
}

// ── Inner implementation (DuckDB-only) ─────────────────────────────────────

#[cfg(feature = "duckdb")]
async fn run_checks_inner(args: CheckArgs) -> Result<()> {
    use smelt_cli::test_runner::batches_to_rows;

    // 1. Find project root and load config.
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    let config = Config::load(&project_dir).with_context(|| "Failed to load smelt.yml")?;

    let target = &args.target;
    let target_config = config.targets.get(target).ok_or_else(|| {
        anyhow::anyhow!(
            "Target '{}' not found in smelt.yml. Available targets: {}",
            target,
            config
                .targets
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;
    let schema = &target_config.schema;

    // 2. Discover all models.
    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let all_models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;

    // 3. Separate check models from regular models.
    let check_models: Vec<_> = all_models.iter().filter(|m| m.is_check()).collect();
    let regular_models: Vec<_> = all_models
        .iter()
        .filter(|m| !m.is_assertion())
        .cloned()
        .collect();

    if check_models.is_empty() {
        println!("No checks found.");
        return Ok(());
    }

    // 4. Apply --select substring filter.
    // The check name is the identifier in the `smelt.check <name> AS (...)` declaration.
    let selected_checks: Vec<_> = if args.select.is_empty() {
        check_models
    } else {
        check_models
            .into_iter()
            .filter(|model| {
                let check_name = extract_check_name_from_content(&model.content)
                    .unwrap_or_else(|| model.name.clone());
                args.select
                    .iter()
                    .any(|sel| check_name.contains(sel.as_str()))
            })
            .collect()
    };

    if selected_checks.is_empty() {
        println!("No checks matched the selection.");
        return Ok(());
    }

    // 5. Build ephemeral resolver from regular (non-assertion) models.
    //    Mirrors the ephemeral-collection logic in `execute_project` (execute.rs):
    //    iterate topological order, collect models whose materialization is Ephemeral.
    let mut ephemeral_models: Vec<(String, String)> = Vec::new();
    if let Ok(graph) = DependencyGraph::build(regular_models.clone(), None) {
        if let Ok(exec_order) = graph.execution_order() {
            for model_name in &exec_order {
                if let Ok(model) = graph.get_model(model_name) {
                    let metadata = model.metadata.as_deref();
                    let mat = config.get_materialization_with_metadata(model_name, metadata);
                    if mat == smelt_core::config::Materialization::Ephemeral {
                        ephemeral_models.push((model_name.clone(), model.content.clone()));
                    }
                }
            }
        }
    }

    // 6. Build CompilerRegistry (sanctioned run-pipeline-parity path).
    let mut targets_map: HashMap<String, smelt_core::config::Target> = HashMap::new();
    targets_map.insert(target.clone(), target_config.clone());
    let mut compilers = CompilerRegistry::new(&config, &targets_map);

    // Wire in function bodies so `smelt.functions.*` calls inside checks expand.
    let fn_files = discovery.discover_function_files().unwrap_or_default();
    let fn_body_map = build_fn_body_map_from_model_files(&fn_files);
    if !fn_body_map.is_empty() {
        compilers.set_function_bodies_all(fn_body_map);
    }

    let compiler = compilers.get(target);
    let ephemeral_resolver = compiler.build_ephemeral_resolver(&ephemeral_models, schema);
    let ephemeral_names: HashSet<String> = ephemeral_resolver.ephemeral_names.clone();

    // 7. Create backend.
    let mut needed: HashSet<String> = HashSet::new();
    needed.insert(target.clone());
    let backend_reg = BackendRegistry::new(
        &config.targets,
        &needed,
        &project_dir,
        args.database.map(|p| project_dir.join(p)).or_else(|| {
            target_config
                .database
                .as_ref()
                .map(|db| project_dir.join(db))
        }),
    )
    .await?;
    let backend = backend_reg.get(target);

    // 8. Run each selected check.
    println!("\nsmelt check\n");

    let mut pass_count = 0usize;
    let mut fail_count = 0usize;
    let mut warn_count = 0usize;

    for check_model in &selected_checks {
        // Parse the check declaration from the model content.
        let clean_body = smelt_parser::strip_frontmatter(&check_model.content);
        let parse = smelt_parser::parse(&clean_body);
        let ast_file_opt = smelt_parser::ast::File::cast(parse.syntax());

        let check = match ast_file_opt.as_ref().and_then(|f| f.checks().next()) {
            Some(c) => c,
            None => {
                println!(
                    "  FAIL  {} — no smelt.check declaration found in file",
                    check_model.name
                );
                fail_count += 1;
                continue;
            }
        };

        let check_name = check.name().unwrap_or_else(|| check_model.name.clone());

        // Determine severity (default: Error).
        let severity: CheckSeverity = check_model
            .metadata
            .as_ref()
            .and_then(|m| m.check.as_ref())
            .map(|c| c.severity.clone())
            .unwrap_or_default();

        // ── CheckTargetNotBuilt pre-check ─────────────────────────────────
        // For each smelt.<path> ref in the check model that is not ephemeral,
        // not a source, and not a function — verify the relation exists in the
        // target before executing the SQL. A missing relation is always a loud
        // error (fail-loud discipline), regardless of `severity`.
        let mut target_not_built: Option<String> = None;
        for ref_info in &check_model.refs {
            let segs = ref_info.smelt_ref.to_path();
            if segs.is_empty() {
                continue;
            }
            // Skip special smelt.<path> namespaces that don't map to built models.
            if segs[0] == "sources" || segs[0] == "functions" {
                continue;
            }
            let relation_name = segs.join("_");
            // Skip ephemeral models — they are inlined as CTEs, never materialised.
            if ephemeral_names.contains(&relation_name) {
                continue;
            }
            match backend.table_exists(schema, &relation_name).await {
                Ok(true) => {}
                Ok(false) => {
                    target_not_built = Some(format!(
                        "CheckTargetNotBuilt: model '{}' referenced by check '{}' \
                         has not been built in target '{}'",
                        segs.join("."),
                        check_name,
                        target
                    ));
                    break;
                }
                Err(e) => {
                    target_not_built = Some(format!(
                        "CheckTargetNotBuilt: error verifying '{}' in target '{}': {}",
                        relation_name, target, e
                    ));
                    break;
                }
            }
        }

        if let Some(msg) = target_not_built {
            println!("  FAIL  {} — {}", check_name, msg);
            fail_count += 1;
            continue;
        }

        // ── Extract the check body SELECT ─────────────────────────────────
        let body_select_text = match check.body_select() {
            Some(s) => s.syntax().text().to_string(),
            None => {
                println!("  FAIL  {} — check has no SELECT body", check_name);
                fail_count += 1;
                continue;
            }
        };

        // ── Compile through the sanctioned CompilerRegistry path ──────────
        // `compile_with_sql_and_ephemerals` translates `smelt.<path>` refs to
        // `schema.relation_name` and inlines any ephemeral CTE dependencies.
        let compiled = match compiler.compile_with_sql_and_ephemerals(
            check_model,
            schema,
            &body_select_text,
            &ephemeral_resolver,
        ) {
            Ok(c) => c,
            Err(e) => {
                println!("  FAIL  {} — compilation error: {}", check_name, e);
                fail_count += 1;
                continue;
            }
        };

        if args.verbose {
            println!("  -- Compiled SQL for {}:", check_name);
            println!("{}", compiled.sql);
        }

        // ── Execute the failing-rows query ────────────────────────────────
        let batches = match backend.execute_sql(&compiled.sql).await {
            Ok(b) => b,
            Err(e) => {
                println!("  FAIL  {} — execution error: {}", check_name, e);
                fail_count += 1;
                continue;
            }
        };

        let row_count: usize = batches.iter().map(|b| b.num_rows()).sum();

        if row_count == 0 {
            // Zero rows = PASS.
            println!("  PASS  {}", check_name);
            pass_count += 1;
        } else {
            // One or more rows = violation.
            let sample = batches_to_rows(&batches);
            let sample_capped: Vec<_> = sample.iter().take(5).collect();

            match severity {
                CheckSeverity::Error => {
                    println!("  FAIL  {} — {} violating row(s)", check_name, row_count);
                    for row in &sample_capped {
                        println!("    {:?}", row);
                    }
                    fail_count += 1;
                }
                CheckSeverity::Warn => {
                    println!("  WARN  {} — {} violating row(s)", check_name, row_count);
                    for row in &sample_capped {
                        println!("    {:?}", row);
                    }
                    warn_count += 1;
                }
            }
        }
    }

    // 9. Summary and exit code.
    let total = pass_count + fail_count + warn_count;
    println!(
        "\n  {} passed, {} failed, {} warned, {} total\n",
        pass_count, fail_count, warn_count, total
    );

    // Exit nonzero iff any error-severity check has violations.
    if fail_count > 0 {
        std::process::exit(1);
    }

    Ok(())
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Extract the check name from the model's content (strip frontmatter, parse AST).
///
/// Returns `None` if the content doesn't contain a valid `smelt.check` declaration.
fn extract_check_name_from_content(content: &str) -> Option<String> {
    let clean = smelt_parser::strip_frontmatter(content);
    let parse = smelt_parser::parse(&clean);
    let file = smelt_parser::ast::File::cast(parse.syntax())?;
    let check = file.checks().next()?;
    check.name()
}
