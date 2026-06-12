use anyhow::{Context, Result};
use smelt_cli::{find_project_root, Config, ModelDiscovery};

use tracing::{debug, warn};

use crate::helpers::{print_property_test_result, print_test_result};
use crate::TestArgs;

#[cfg(feature = "duckdb")]
pub async fn run_tests(args: TestArgs) -> Result<()> {
    use smelt_cli::test_compiler::{
        compile_cte_test, compile_whole_model_test, validate_test_expect,
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
    let regular_models: Vec<_> = models.iter().filter(|m| !m.is_test()).collect();

    if test_models.is_empty() {
        println!("No tests found.");
        return Ok(());
    }

    // 4. Apply selection filter
    let selected_tests: Vec<_> = if args.select.is_empty() {
        test_models
    } else {
        test_models
            .into_iter()
            .filter(|m| args.select.iter().any(|s| m.name.contains(s)))
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
        let test_config = match test_model.test_config() {
            Some(tc) => tc,
            None => {
                warn!("SKIP {} (missing test configuration)", test_model.name);
                continue;
            }
        };

        // Validate config: `expect` is required (spec Constraint-3).
        if let Some(err_msg) = validate_test_expect(&test_config.expect) {
            let result = smelt_cli::test_runner::TestResult {
                name: test_model.name.clone(),
                model: test_config.model.clone(),
                target_cte: test_config.target_cte.clone(),
                passed: false,
                duration: std::time::Duration::from_secs(0),
                compiled_sql: String::new(),
                error: Some(TestError::CompilationError(err_msg)),
            };
            failed += 1;
            if !args.json {
                print_test_result(&result, args.verbose, args.show_all);
            }
            results.push(result);
            continue;
        }

        // Find the model being tested
        let target_model = regular_models.iter().find(|m| m.name == test_config.model);

        let model_sql = match target_model {
            Some(m) => &m.content,
            None => {
                let result = smelt_cli::test_runner::TestResult {
                    name: test_model.name.clone(),
                    model: test_config.model.clone(),
                    target_cte: test_config.target_cte.clone(),
                    passed: false,
                    duration: std::time::Duration::from_secs(0),
                    compiled_sql: String::new(),
                    error: Some(TestError::CompilationError(format!(
                        "Model '{}' not found",
                        test_config.model
                    ))),
                };
                failed += 1;
                if !args.json {
                    print_test_result(&result, args.verbose, args.show_all);
                }
                results.push(result);
                continue;
            }
        };

        // Get the test's own SQL body (if any — for advanced tests)
        let test_sql_body = {
            let clean = smelt_parser::strip_frontmatter(&test_model.content);
            let trimmed = clean.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        };

        // Property-test dispatch: when the CTE body references columns absent from
        // inputs, skip the single-compile path — run_property_test recompiles with
        // augmented inputs on each iteration.
        {
            use smelt_cli::test_property::try_dispatch_property_test;
            if let Some(prop_result) = try_dispatch_property_test(
                &test_model.name,
                &test_config.model,
                test_config.target_cte.as_deref(),
                model_sql,
                test_config,
            ) {
                if prop_result.passed {
                    passed += 1;
                } else {
                    failed += 1;
                }
                if !args.json {
                    print_property_test_result(&prop_result, args.verbose, args.show_all);
                }
                continue;
            }
        }

        // Compile the test
        let compiled_sql = if let Some(ref target_cte) = test_config.target_cte {
            // CTE test
            match compile_cte_test(
                model_sql,
                target_cte,
                &test_config.inputs,
                test_sql_body.as_deref(),
            ) {
                Ok(sql) => sql,
                Err(e) => {
                    let result = smelt_cli::test_runner::TestResult {
                        name: test_model.name.clone(),
                        model: test_config.model.clone(),
                        target_cte: Some(target_cte.clone()),
                        passed: false,
                        duration: std::time::Duration::from_secs(0),
                        compiled_sql: String::new(),
                        error: Some(TestError::CompilationError(e)),
                    };
                    failed += 1;
                    if !args.json {
                        print_test_result(&result, args.verbose, args.show_all);
                    }
                    results.push(result);
                    continue;
                }
            }
        } else {
            // Whole-model test.  When function bodies are available, expand
            // `smelt.functions.*` call nodes so the generated SQL executes
            // cleanly in DuckDB without the `smelt` catalog present.
            let compiled = if fn_body_map.is_empty() {
                compile_whole_model_test(model_sql, &test_config.inputs, test_sql_body.as_deref())
            } else {
                use smelt_cli::compile_whole_model_test_with_fns;
                compile_whole_model_test_with_fns(
                    model_sql,
                    &test_config.inputs,
                    test_sql_body.as_deref(),
                    &fn_body_map,
                )
            };
            match compiled {
                Ok(sql) => sql,
                Err(e) => {
                    let result = smelt_cli::test_runner::TestResult {
                        name: test_model.name.clone(),
                        model: test_config.model.clone(),
                        target_cte: None,
                        passed: false,
                        duration: std::time::Duration::from_secs(0),
                        compiled_sql: String::new(),
                        error: Some(TestError::CompilationError(e)),
                    };
                    failed += 1;
                    if !args.json {
                        print_test_result(&result, args.verbose, args.show_all);
                    }
                    results.push(result);
                    continue;
                }
            }
        };

        if args.verbose {
            debug!("Compiled SQL for {}:\n{}", test_model.name, compiled_sql);
        }

        // Run the test
        let check_order = test_config.check_order.unwrap_or(false);
        let result = run_test(
            &test_model.name,
            &test_config.model,
            test_config.target_cte.as_deref(),
            &compiled_sql,
            &test_config.expect,
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
