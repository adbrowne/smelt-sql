use anyhow::{Context, Result};
use smelt_cli::{
    argument_resolution::resolve_selector_args, find_project_root, Config, ModelDiscovery,
};
use smelt_core::graph::DependencyGraph;
use std::collections::HashMap;

use tracing::{debug, warn};

use crate::helpers::{print_property_test_result, print_test_result};
use crate::TestArgs;

/// Format a `TestInliningError` as an anchored diagnostic string for terminal
/// output, e.g.:
/// ```text
/// /path/to/tests/test_ambiguous.sql:2:25: error[AmbiguousTestModel]: ...
/// ```
///
/// Computes the file-level byte offset of the offending ref by adding
/// `body_select_file_start` (the offset of the body SELECT within the file)
/// to the ref's body-relative offset from `e.body_ref_range.0`, then
/// converts to `(line, col)` via `LineIndex`.
#[cfg(feature = "duckdb")]
fn format_inlining_diagnostic(
    e: &smelt_cli::test_compiler::TestInliningError,
    test_model: &smelt_cli::ModelFile,
    body_select_file_start: usize,
) -> String {
    use line_index::{LineIndex, TextSize};
    let file_offset = body_select_file_start + e.body_ref_range.0;
    let line_index = LineIndex::new(&test_model.content);
    let lc = line_index.line_col(TextSize::from(file_offset as u32));
    format!(
        "{}:{}:{}: error[{}]: {}",
        test_model.path.display(),
        lc.line + 1,
        lc.col + 1,
        e.code,
        e.message
    )
}

#[cfg(feature = "duckdb")]
pub async fn run_tests(args: TestArgs) -> Result<()> {
    use smelt_cli::test_compiler::{
        compile_cte_test, compile_whole_query_test, find_cte_ref_in_body,
        record_literal_to_yaml_row,
    };
    use smelt_cli::test_runner::{run_test, TestError};
    use std::time::Instant;

    let overall_start = Instant::now();

    // 1. Find project root
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    // 2. Load configuration and discover models
    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;

    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;

    // 2b. Discover function files and build a FnBodyMap for test-time expansion.
    // This allows test SQL that calls `smelt.functions.*` to be expanded before
    // execution — mirroring what `smelt build` does at print time.
    let fn_body_map = {
        let fn_files = discovery.discover_function_files().unwrap_or_default();
        smelt_runtime::build_fn_body_map_from_model_files(&fn_files)
    };

    // 3. Separate test models from regular models
    let test_models: Vec<_> = models.iter().filter(|m| m.is_test()).collect();
    // The non-test pool excludes all assertion files (tests AND checks): a check
    // is not an inlinable model body and must not appear in the test selector graph.
    let regular_models: Vec<_> = models.iter().filter(|m| !m.is_assertion()).collect();

    // Whole-query test inlining inputs (testing.md §Execution model):
    //   * `canonical_bodies` — every regular model's frontmatter-stripped body,
    //     keyed by its dotted canonical address.
    //   * `leaf_to_canonicals` — each model leaf name → the canonical addresses
    //     that share it, so a single-segment ref to an ambiguous leaf fails loud
    //     instead of resolving to whichever model was discovered first.
    let canonical_bodies: std::collections::BTreeMap<String, String> = regular_models
        .iter()
        .map(|m| {
            (
                m.canonical_path(),
                smelt_parser::strip_frontmatter(&m.content).to_string(),
            )
        })
        .collect();
    let leaf_to_canonicals: std::collections::BTreeMap<String, Vec<String>> = {
        let mut map: std::collections::BTreeMap<String, Vec<String>> =
            std::collections::BTreeMap::new();
        for m in &regular_models {
            map.entry(m.name.clone())
                .or_default()
                .push(m.canonical_path());
        }
        for v in map.values_mut() {
            v.sort();
            v.dedup();
        }
        map
    };

    if test_models.is_empty() {
        println!("No tests found.");
        return Ok(());
    }

    // 4. Apply selection filter using full selector syntax (D-41).
    //
    // Selection targets the REGULAR models (models under test), not test-model
    // names: `--select tag:X` runs tests whose subject model carries tag X;
    // `--select +model` runs tests for model and its transitive upstreams.
    // Entity selectors that resolve to no model are a hard "not found" error
    // (non-zero); method selectors (tag:, generator_file:) that match nothing
    // are a valid empty selection (exit 0, "no tests to run" message).
    let selected_tests: Vec<_> = if args.select.is_empty() {
        test_models
    } else {
        // Build Salsa DB for canonical address resolution and smelt.-strip (D-36).
        let all_models = discovery.discover_models().unwrap_or_default();
        let salsa_db = smelt_cli::init_db(&project_dir, &all_models);
        let salsa_ws = smelt_db::Workspace::try_get(&salsa_db).expect("workspace not initialized");
        let salsa_proj = salsa_db
            .project_input(&project_dir)
            .expect("project not initialized");
        let resolved_select =
            resolve_selector_args(&salsa_db, salsa_ws, salsa_proj, None, &args.select)
                .map_err(|e| anyhow::anyhow!("{}", e))?;

        // Build a dependency graph from regular models only — exclude all
        // assertion files (tests AND checks); a check is not part of the graph.
        let regular_model_files: Vec<smelt_cli::ModelFile> = all_models
            .into_iter()
            .filter(|m| !m.is_assertion())
            .collect();
        // Map leaf-name → canonical path so we can match test_config.model.
        let leaf_to_canonical: HashMap<String, String> = regular_model_files
            .iter()
            .map(|m| (m.name.clone(), m.canonical_path()))
            .collect();
        let graph = DependencyGraph::build(regular_model_files, None)?;

        let selectors: Vec<_> = resolved_select
            .iter()
            .map(|s| {
                smelt_core::parse_selector(s).with_context(|| format!("Invalid selector '{}'", s))
            })
            .collect::<Result<_>>()?;

        // Hard error for unresolvable entity selectors; empty set for method
        // selectors that match nothing (handled as a no-op below).
        let selected_models = graph
            .select_models(&selectors, &config)
            .context("smelt test --select: selector failed")?;

        if selected_models.is_empty() {
            eprintln!("No models matched the selector(s). No tests to run.");
            return Ok(());
        }

        // Filter test models: include only those whose target model's canonical
        // path is in the selected set.
        //
        // Subject models are derived from the smelt.<path> refs in each
        // smelt.test declaration's assertion body (new-syntax path).
        test_models
            .into_iter()
            .filter(|t| {
                smelt_cli::test_compiler::new_syntax_test_subject_model_leaves(&t.content)
                    .iter()
                    .any(|leaf| {
                        leaf_to_canonical
                            .get(leaf)
                            .map(|cp| selected_models.contains(cp))
                            .unwrap_or(false)
                    })
            })
            .collect()
    };

    if selected_tests.is_empty() {
        println!("No tests matched the selection.");
        return Ok(());
    }

    if !args.json {
        println!("\nsmelt test\n");
    }

    // 5. Run each test
    let mut passed = 0;
    let mut failed = 0;
    let mut results = Vec::new();

    for test_model in &selected_tests {
        // ── New path: smelt.test AST-driven declarations ─────────────────────
        {
            let clean_body = smelt_parser::strip_frontmatter(&test_model.content);
            let parse_result = smelt_parser::parse(&clean_body);
            let ast_file_opt = smelt_parser::ast::File::cast(parse_result.syntax());
            if let Some(ast_file) = &ast_file_opt {
                let smelt_tests: Vec<_> = ast_file.tests().collect();
                if !smelt_tests.is_empty() {
                    for smelt_test in smelt_tests {
                        let test_name =
                            smelt_test.name().unwrap_or_else(|| test_model.name.clone());

                        // Build inputs from PASSING clauses.
                        let mut inputs: std::collections::BTreeMap<
                            String,
                            Vec<std::collections::BTreeMap<String, serde_yaml::Value>>,
                        > = std::collections::BTreeMap::new();
                        for clause in smelt_test.passing_clauses() {
                            let clause_name = match clause.name() {
                                Some(n) => n,
                                None => continue,
                            };
                            let rows: Vec<_> = clause
                                .rows()
                                .map(|r| record_literal_to_yaml_row(&r))
                                .collect();
                            inputs.insert(clause_name, rows);
                        }

                        // Build expect rows from EXPECT clause.
                        let expect_rows: Vec<
                            std::collections::BTreeMap<String, serde_yaml::Value>,
                        > = smelt_test
                            .expect_clause()
                            .map(|ec| ec.rows().map(|r| record_literal_to_yaml_row(&r)).collect())
                            .unwrap_or_default();

                        if expect_rows.is_empty() {
                            let result = smelt_cli::test_runner::TestResult {
                                name: test_name.clone(),
                                model: test_model.name.clone(),
                                target_cte: None,
                                passed: false,
                                duration: std::time::Duration::from_secs(0),
                                compiled_sql: String::new(),
                                error: Some(TestError::CompilationError(
                                    "smelt.test has no EXPECT clause — at least one \
                                     expected row is required"
                                        .to_string(),
                                )),
                            };
                            failed += 1;
                            if !args.json {
                                print_test_result(&result, args.verbose, args.show_all);
                            }
                            results.push(result);
                            continue;
                        }

                        // check_order and cases from frontmatter `test:` block.
                        let check_order = test_model
                            .metadata
                            .as_ref()
                            .and_then(|m| m.test.as_ref())
                            .and_then(|t| t.check_order)
                            .unwrap_or(false);
                        let cases_count = test_model
                            .metadata
                            .as_ref()
                            .and_then(|m| m.test.as_ref())
                            .and_then(|t| t.cases)
                            .unwrap_or(10);

                        // Get the body SELECT text and its start offset within the
                        // file (for anchoring inlining diagnostics to file:line:col).
                        let (body_select, body_select_file_start) = match smelt_test.body_select() {
                            Some(s) => {
                                let file_start: usize = s.syntax().text_range().start().into();
                                (s.syntax().text().to_string(), file_start)
                            }
                            None => {
                                let result = smelt_cli::test_runner::TestResult {
                                    name: test_name.clone(),
                                    model: test_model.name.clone(),
                                    target_cte: None,
                                    passed: false,
                                    duration: std::time::Duration::from_secs(0),
                                    compiled_sql: String::new(),
                                    error: Some(TestError::CompilationError(
                                        "smelt.test has no body SELECT".to_string(),
                                    )),
                                };
                                failed += 1;
                                if !args.json {
                                    print_test_result(&result, args.verbose, args.show_all);
                                }
                                results.push(result);
                                continue;
                            }
                        };

                        // Detect whether the body contains a `smelt.<path>#<cte>` ref.
                        let cte_ref = find_cte_ref_in_body(&body_select);

                        if let Some((model_segs, cte_name)) = cte_ref {
                            // ── CTE-level test ──────────────────────────────────────────────
                            // Find the referenced model in the project.
                            let model_canonical = model_segs.join(".");
                            let model_leaf = model_segs.last().cloned().unwrap_or_default();
                            let model_file = regular_models.iter().find(|m| {
                                m.name == model_leaf || m.canonical_path() == model_canonical
                            });
                            match model_file {
                                Some(mf) => {
                                    // Detect omitted PASSING columns → property-based dispatch.
                                    // When the target CTE body references columns absent from
                                    // `inputs`, run `cases` iterations with random augmented rows
                                    // (mirrors the legacy property-test path in the legacy runner).
                                    use smelt_cli::test_property::find_missing_columns;
                                    let missing =
                                        find_missing_columns(&mf.content, &cte_name, &inputs);
                                    if !missing.is_empty() {
                                        use smelt_cli::test_property::run_property_test;
                                        let prop_result = run_property_test(
                                            &test_name,
                                            &model_leaf,
                                            Some(&cte_name),
                                            &mf.content,
                                            &inputs,
                                            &expect_rows,
                                            check_order,
                                            cases_count,
                                            None,
                                        );
                                        if prop_result.passed {
                                            passed += 1;
                                        } else {
                                            failed += 1;
                                        }
                                        if !args.json {
                                            print_property_test_result(
                                                &prop_result,
                                                args.verbose,
                                                args.show_all,
                                            );
                                        }
                                        // Property test results are not pushed to `results`
                                        // (matches the legacy property-test path behaviour).
                                        continue;
                                    }

                                    // One-shot CTE test (all columns provided).
                                    match compile_cte_test(&mf.content, &cte_name, &inputs, None) {
                                        Ok(compiled_sql) => {
                                            if args.verbose {
                                                debug!(
                                                    "Compiled SQL for {}:\n{}",
                                                    test_name, compiled_sql
                                                );
                                            }
                                            let result = run_test(
                                                &test_name,
                                                &model_leaf,
                                                Some(&cte_name),
                                                &compiled_sql,
                                                &expect_rows,
                                                check_order,
                                            );
                                            if result.passed {
                                                passed += 1;
                                            } else {
                                                failed += 1;
                                            }
                                            if !args.json {
                                                print_test_result(
                                                    &result,
                                                    args.verbose,
                                                    args.show_all,
                                                );
                                            }
                                            results.push(result);
                                        }
                                        Err(e) => {
                                            let result = smelt_cli::test_runner::TestResult {
                                                name: test_name.clone(),
                                                model: model_leaf.clone(),
                                                target_cte: Some(cte_name.clone()),
                                                passed: false,
                                                duration: std::time::Duration::from_secs(0),
                                                compiled_sql: String::new(),
                                                error: Some(TestError::CompilationError(e)),
                                            };
                                            failed += 1;
                                            if !args.json {
                                                print_test_result(
                                                    &result,
                                                    args.verbose,
                                                    args.show_all,
                                                );
                                            }
                                            results.push(result);
                                        }
                                    }
                                }
                                None => {
                                    let result = smelt_cli::test_runner::TestResult {
                                        name: test_name.clone(),
                                        model: model_canonical.clone(),
                                        target_cte: Some(cte_name.clone()),
                                        passed: false,
                                        duration: std::time::Duration::from_secs(0),
                                        compiled_sql: String::new(),
                                        error: Some(TestError::CompilationError(format!(
                                            "UnknownTestCte: model '{}' not found \
                                             in project",
                                            model_canonical
                                        ))),
                                    };
                                    failed += 1;
                                    if !args.json {
                                        print_test_result(&result, args.verbose, args.show_all);
                                    }
                                    results.push(result);
                                }
                            }
                            // Done with this CTE-level smelt.test — skip full-query path.
                            continue;
                        }

                        // ── Full-query test (no #cte ref) ───────────────────────────────────
                        // Inline every referenced model that is not directly mocked via
                        // PASSING, so the assertion runs against the real model output and
                        // the model's upstream deps become the mockable PASSING inputs
                        // (testing.md §Execution model — "inlining the body of every model
                        // it references"). Self-contained bodies and bodies that read their
                        // deps directly are handled by the same path.
                        let compiled_result = compile_whole_query_test(
                            &body_select,
                            &inputs,
                            &canonical_bodies,
                            &leaf_to_canonicals,
                            Some(&fn_body_map),
                        );
                        match compiled_result {
                            Ok(compiled_sql) => {
                                if args.verbose {
                                    debug!("Compiled SQL for {}:\n{}", test_name, compiled_sql);
                                }
                                let result = run_test(
                                    &test_name,
                                    &test_model.name,
                                    None,
                                    &compiled_sql,
                                    &expect_rows,
                                    check_order,
                                );
                                if result.passed {
                                    passed += 1;
                                } else {
                                    failed += 1;
                                }
                                if !args.json {
                                    print_test_result(&result, args.verbose, args.show_all);
                                }
                                results.push(result);
                            }
                            Err(e) => {
                                let anchored_msg = format_inlining_diagnostic(
                                    &e,
                                    test_model,
                                    body_select_file_start,
                                );
                                let result = smelt_cli::test_runner::TestResult {
                                    name: test_name.clone(),
                                    model: test_model.name.clone(),
                                    target_cte: None,
                                    passed: false,
                                    duration: std::time::Duration::from_secs(0),
                                    compiled_sql: String::new(),
                                    error: Some(TestError::InliningDiagnostic {
                                        code: e.code,
                                        message: anchored_msg,
                                    }),
                                };
                                failed += 1;
                                if !args.json {
                                    print_test_result(&result, args.verbose, args.show_all);
                                }
                                results.push(result);
                            }
                        }
                    }
                    // All smelt.test declarations in this file are handled above.
                    // Skip the legacy path.
                    continue;
                }
            }
        }
        // ── End smelt.test path ──────────────────────────────────────────────
        // A test file with no smelt.test declarations is skipped (warn).
        warn!(
            "SKIP {} (no smelt.test declarations found)",
            test_model.name
        );
    }

    // 6. Output results
    if args.json {
        #[derive(serde::Serialize)]
        struct JsonTestResult {
            name: String,
            model: String,
            status: &'static str,
            duration_ms: u64,
            message: Option<String>,
        }

        #[derive(serde::Serialize)]
        struct JsonOutput {
            results: Vec<JsonTestResult>,
        }

        let json_results: Vec<JsonTestResult> = results
            .into_iter()
            .map(|r| JsonTestResult {
                status: if r.passed { "pass" } else { "fail" },
                duration_ms: r.duration.as_millis() as u64,
                message: r.error.map(|e| e.to_string()),
                name: r.name,
                model: r.model,
            })
            .collect();

        let output = JsonOutput {
            results: json_results,
        };
        let json = serde_json::to_string(&output)?;
        use std::io::Write;
        writeln!(std::io::stdout(), "{json}")?;
        return Ok(());
    }

    let total = passed + failed;
    let overall_duration = overall_start.elapsed();
    println!(
        "\n  {} passed, {} failed, {} total ({:.2}s)\n",
        passed,
        failed,
        total,
        overall_duration.as_secs_f64()
    );

    if failed > 0 {
        std::process::exit(1);
    }

    Ok(())
}

#[cfg(not(feature = "duckdb"))]
pub async fn run_tests(_args: TestArgs) -> Result<()> {
    Err(anyhow::anyhow!(
        "The 'test' command requires the 'duckdb' feature. Build with: cargo build --features duckdb"
    ))
}
