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
    init_db, CompilerRegistry, Config, ModelDiscovery,
};
use smelt_core::{
    graph::DependencyGraph,
    metadata::{CheckSeverity, ColumnTest},
    ModelFile, ModelId, ModelKind,
};
use smelt_logical::{
    lower_column_test, resolve_not_null_verdict, resolve_unique_verdict, ScanLowering, TestVerdict,
};
use smelt_runtime::{run_single_check, CheckStatus};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::CheckArgs;

/// One unproven declarative column test, lowered and ready to execute
/// through the same `run_single_check` machinery a hand-authored
/// `smelt.check` uses (`docs/specs/data_tests.md` §Semantics step 2).
struct PendingScan {
    model_name: String,
    column: String,
    lowering: ScanLowering,
}

/// A declarative column test whose shape `lower_column_test` could not lower
/// (defensive — expected to be unreachable for tests that already passed
/// `validate_column_tests`, but fail-loud rather than silently skipped;
/// `docs/specs/data_tests.md` §"Fail-loud validation").
struct UnlowerableTest {
    model_name: String,
    column: String,
    message: String,
}

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

    // Declarative column tests (`columns.<c>.tests`, `docs/specs/data_tests.md`)
    // — proven-verdict short-circuit plus scan lowering for everything left
    // unproven. Runs unconditionally (independent of whether the project has
    // any `smelt.check` declarations or a `--select` match), since a project
    // may declare column tests without ever declaring a hand-authored check.
    //
    // Consult derived properties first (inferred nullability for `not_null`,
    // the declared `unique_key:` for `unique`); a proven verdict is reported
    // here with no scan. Every other test — `accepted_values`/`relationships`
    // (no proof path exists for either today), and any `not_null`/`unique`
    // the proof step could not decide — lowers to a failing-rows scan
    // (`smelt_logical::lower_column_test`) collected into `pending_scans` and
    // driven through the same `run_single_check` machinery as the
    // `smelt.check` loop below, once the compiler/backend are ready.
    let mut proven_count = 0usize;
    let mut pending_scans: Vec<PendingScan> = Vec::new();
    let mut unlowerable: Vec<UnlowerableTest> = Vec::new();
    if regular_models
        .iter()
        .any(|m| model_has_column_tests(m.metadata.as_deref()))
    {
        let db = init_db(&project_dir, &all_models);
        if let Some(ws) = smelt_db::Workspace::try_get(&db) {
            println!("\nsmelt check — declarative column tests\n");
            for model in &regular_models {
                let Some(metadata) = model.metadata.as_deref() else {
                    continue;
                };
                if !model_has_column_tests(Some(metadata)) {
                    continue;
                }
                let Some(file) = db.source_file(&model.path) else {
                    continue;
                };
                let typed_schema = smelt_db::typed_model_schema(&db, ws, file);
                let known_key_sets: Vec<Vec<String>> = metadata
                    .unique_key
                    .as_ref()
                    .map(|k| vec![k.clone()])
                    .unwrap_or_default();

                let mut columns: Vec<_> = metadata.columns.iter().collect();
                columns.sort_by(|a, b| a.0.cmp(b.0));
                for (column, col_meta) in columns {
                    for test in &col_meta.tests {
                        // Only `not_null`/`unique` have a proof step
                        // (`docs/specs/data_tests.md` §"Known Divergences" —
                        // `accepted_values`/`relationships` are always
                        // unproven). An undecidable proof is fail-safe:
                        // it falls through to `NeedsScan`, never a claimed
                        // pass.
                        let verdict = match test {
                            ColumnTest::Simple(kind) if kind == "not_null" => {
                                let is_non_nullable = column_non_nullable(&typed_schema, column);
                                Some(resolve_not_null_verdict(is_non_nullable))
                            }
                            ColumnTest::Simple(kind) if kind == "unique" => {
                                Some(resolve_unique_verdict(
                                    std::slice::from_ref(column),
                                    &known_key_sets,
                                ))
                            }
                            _ => None,
                        };

                        if verdict == Some(TestVerdict::Proven) {
                            let label = match test {
                                ColumnTest::Simple(kind) => kind.as_str(),
                                ColumnTest::Parameterized(_) => "unknown",
                            };
                            println!(
                                "  PROVEN  {}.{}.{} — no scan emitted",
                                model.name, column, label
                            );
                            proven_count += 1;
                            continue;
                        }

                        match lower_column_test(&model.name, column, test) {
                            Ok(lowering) => pending_scans.push(PendingScan {
                                model_name: model.name.clone(),
                                column: column.clone(),
                                lowering,
                            }),
                            Err(message) => unlowerable.push(UnlowerableTest {
                                model_name: model.name.clone(),
                                column: column.clone(),
                                message,
                            }),
                        }
                    }
                }
            }
            if proven_count > 0 {
                println!();
            }
        }
    }

    if check_models.is_empty() && pending_scans.is_empty() && unlowerable.is_empty() {
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

    if selected_checks.is_empty() && pending_scans.is_empty() && unlowerable.is_empty() {
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
    let ephemeral_resolver = compiler.build_ephemeral_resolver(&ephemeral_models, schema)?;
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
        // Determine severity (default: Error).
        let severity: CheckSeverity = check_model
            .metadata
            .as_ref()
            .and_then(|m| m.check.as_ref())
            .map(|c| c.severity.clone())
            .unwrap_or_default();

        // Compile + execute through the shared runtime helper. Run-pipeline
        // parity: the compile/execute path lives in `smelt-runtime`, so the
        // same `run_single_check` drives both `smelt check` and `smelt build`'s
        // build-time check pass — neither duplicates the execute logic.
        let outcome = run_single_check(
            compiler,
            backend,
            schema,
            check_model,
            severity,
            &ephemeral_names,
            &ephemeral_resolver,
        )
        .await?;

        if args.verbose {
            if let Some(sql) = &outcome.sql {
                println!("  -- Compiled SQL for {}:", outcome.name);
                println!("{}", sql);
            }
        }

        match outcome.status {
            CheckStatus::Pass => {
                println!("  PASS  {}", outcome.name);
                pass_count += 1;
            }
            CheckStatus::Fail => {
                let detail = outcome.message.as_deref().unwrap_or("violation");
                println!("  FAIL  {} — {}", outcome.name, detail);
                for row in &outcome.sample {
                    println!("    {:?}", row);
                }
                fail_count += 1;
            }
            CheckStatus::Warn => {
                let detail = outcome.message.as_deref().unwrap_or("violation");
                println!("  WARN  {} — {}", outcome.name, detail);
                for row in &outcome.sample {
                    println!("    {:?}", row);
                }
                warn_count += 1;
            }
            CheckStatus::TargetNotBuilt => {
                let detail = outcome.message.as_deref().unwrap_or("CheckTargetNotBuilt");
                println!("  FAIL  {} — {}", outcome.name, detail);
                fail_count += 1;
            }
        }
    }

    // 8b. Run every unproven declarative column test's lowered scan through
    // the same `run_single_check` machinery (run-pipeline parity — no second
    // execution path). Each scan is wrapped as a synthetic error-severity
    // `smelt.check` (`docs/specs/data_tests.md` §Semantics: declarative
    // column tests are error-severity only).
    for pending in &pending_scans {
        let label = format!(
            "{}.{}.{}",
            pending.model_name, pending.column, pending.lowering.kind
        );
        let check_ident = format!(
            "{}__{}__{}",
            pending.model_name, pending.column, pending.lowering.kind
        );
        let check_model = build_scan_check_model(&check_ident, &pending.lowering.failing_rows_sql);

        let outcome = run_single_check(
            compiler,
            backend,
            schema,
            &check_model,
            CheckSeverity::Error,
            &ephemeral_names,
            &ephemeral_resolver,
        )
        .await?;

        if args.verbose {
            if let Some(sql) = &outcome.sql {
                println!("  -- Compiled SQL for {label}:");
                println!("{}", sql);
            }
        }

        match outcome.status {
            CheckStatus::Pass => {
                println!("  PASS  {label}");
                pass_count += 1;
            }
            CheckStatus::Fail => {
                let detail = outcome.message.as_deref().unwrap_or("violation");
                println!("  FAIL  {label} — {detail}");
                for row in &outcome.sample {
                    println!("    {:?}", row);
                }
                fail_count += 1;
            }
            CheckStatus::Warn => {
                // Declarative column tests are error-severity only
                // (`docs/specs/data_tests.md` §Semantics "Severity") —
                // `run_single_check` never returns `Warn` for the
                // `CheckSeverity::Error` we always pass above, but handle it
                // rather than silently drop a status this match must cover.
                let detail = outcome.message.as_deref().unwrap_or("violation");
                println!("  WARN  {label} — {detail}");
                for row in &outcome.sample {
                    println!("    {:?}", row);
                }
                warn_count += 1;
            }
            CheckStatus::TargetNotBuilt => {
                let detail = outcome.message.as_deref().unwrap_or("CheckTargetNotBuilt");
                println!("  FAIL  {label} — {detail}");
                fail_count += 1;
            }
        }
    }

    // A test `lower_column_test` could not lower is a hard failure, not a
    // silent skip (fail-loud discipline) — this is expected to be
    // unreachable for tests that already passed `validate_column_tests`.
    for bad in &unlowerable {
        println!(
            "  FAIL  {}.{}.<test> — {}",
            bad.model_name, bad.column, bad.message
        );
        fail_count += 1;
    }

    // 9. Summary and exit code.
    let total = pass_count + fail_count + warn_count;
    println!(
        "\n  {} passed, {} failed, {} warned, {} total\n",
        pass_count, fail_count, warn_count, total
    );

    // Exit nonzero iff any error-severity check has violations.
    if fail_count > 0 {
        return Err(
            smelt_cli::CliError::DetectedFailure(format!("{fail_count} check(s) failed")).into(),
        );
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

/// True if any `columns:` entry declares a non-empty `tests` list
/// (`docs/specs/data_tests.md` §Surface).
fn model_has_column_tests(metadata: Option<&smelt_core::metadata::ModelMetadata>) -> bool {
    metadata
        .map(|m| m.columns.values().any(|c| !c.tests.is_empty()))
        .unwrap_or(false)
}

/// Whether `column` is provably non-nullable from the model's own inferred
/// schema (`docs/specs/data_tests.md` §Semantics "Resolution order" — the
/// `not_null` proof step).
///
/// Only trusts `Computed` columns (the model directly determines the
/// value) — pass-through (`FromModel`), wildcard-expanded, and
/// unresolved-source columns are left `None` (undecidable), matching the
/// same reliability filter `check_timeseries_nullability` applies
/// (`crates/smelt-db/src/queries/check_types.rs`). `None` is fail-safe: the
/// caller's [`resolve_not_null_verdict`] treats it as "needs a scan", never
/// as a claimed proof.
fn column_non_nullable(schema: &smelt_db::ModelSchema, column: &str) -> Option<bool> {
    let col = schema.columns.iter().find(|c| c.name == column)?;
    if !matches!(col.source, smelt_db::schema::ColumnSource::Computed) {
        return None;
    }
    col.data_type.as_ref().map(|tc| !tc.nullable)
}

/// Build a synthetic `ModelFile` wrapping one lowered declarative-test scan
/// as a `smelt.check <check_ident> AS (<failing_rows_sql>)` declaration, so
/// it can be driven through `run_single_check` exactly like a hand-authored
/// check (run-pipeline parity rule, `docs/specs/architecture.md` §"Run
/// pipeline parity rule (CLI ↔ UI)").
///
/// `refs` is computed by parsing the generated content through the same
/// `smelt_core::extract_refs` every real model file uses — not hand-authored
/// — so `CheckTargetNotBuilt` detection and ephemeral-CTE inlining see
/// exactly the same references a hand-written check with this body would
/// produce.
fn build_scan_check_model(check_ident: &str, failing_rows_sql: &str) -> ModelFile {
    let content = format!("smelt.check {check_ident} AS (\n{failing_rows_sql}\n)\n");
    let parse = smelt_parser::parse(&content);
    let refs = smelt_parser::ast::File::cast(parse.syntax())
        .map(|file| smelt_core::extract_refs(&file))
        .unwrap_or_default();
    let path = PathBuf::from(format!("<declarative-test>/{check_ident}.sql"));
    ModelFile {
        name: check_ident.to_string(),
        path: path.clone(),
        content,
        refs,
        parse_errors: parse.errors,
        metadata: None,
        kind: ModelKind::Sql,
        model_id: ModelId::from_path(path),
        address_segments: vec![check_ident.to_string()],
    }
}
