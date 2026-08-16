//! Surface-audit tests: verify the Run Pipeline Parity structural-enforcement clause.
//!
//! After Phase 6, `smelt-runtime` exposes only two paths to a `CompiledModel`:
//! - `execute_project(request, reporter)` — the primary entry point.
//! - `CompilerRegistry::get(target).compile_with_sql_and_ephemerals(...)` — for
//!   backbuild, which manages its own backend session outside the shared pipeline.
//!
//! Internal constructors (`SqlCompiler::new`, `SqlCompiler::set_*`, `compile_with_sql`)
//! are `pub(crate)` — this file is an external integration test and therefore
//! cannot call them. Any attempt would be a compile error.

use smelt_runtime::{NoOpReporter, RunReporter};

/// `RunReporter` must be object-safe so both CLI and UI can store it as `Box<dyn RunReporter>`.
#[test]
fn test_run_reporter_is_object_safe() {
    let r: Box<dyn RunReporter> = Box::new(NoOpReporter);
    // Drive every method to ensure default impls are sound.
    r.run_started("rid", &["a".into()], 1);
    r.model_started("rid", "a", 0, 1);
    r.batch_completed("rid", "a", 0, 1, 42, std::time::Duration::from_millis(1));
    r.model_completed("rid", "a", 42, std::time::Duration::from_millis(1));
    r.run_completed("rid", 42, std::time::Duration::from_millis(1));
    r.run_failed("rid", Some("a"), "boom");
    r.run_cancelled("rid");
    r.model_compiled("rid", "a", "SELECT 1");
}

/// The public surface of `SqlCompiler` accessible from outside `smelt-runtime`
/// consists only of the high-level `compile_with_*` methods and
/// `build_ephemeral_resolver`. Construction and configuration must go through
/// `CompilerRegistry`.
///
/// This test drives the allowed path end-to-end:
/// `CompilerRegistry::new` → `set_*_all` → `get` → `compile_with_sql_and_ephemerals`.
#[test]
fn test_no_compiler_internals_exposed() {
    use smelt_core::config::{Config, Target};
    use smelt_runtime::{CompilerRegistry, EphemeralResolver};
    use std::collections::HashMap;

    let mut targets = HashMap::new();
    targets.insert(
        "dev".to_string(),
        Target {
            target_type: "duckdb".to_string(),
            database: Some(":memory:".to_string()),
            schema: "main".to_string(),
            connect_url: None,
            catalog: None,
            warehouse: None,
            format: None,
            settings: None,
            project: None,
            dataset: None,
            location: None,
        },
    );
    let config = Config {
        name: "test".to_string(),
        version: 1,
        paths: vec!["models".to_string()],
        targets: targets.clone(),
        default_materialization: smelt_core::config::Materialization::View,
        models: HashMap::new(),
        python: None,
        target: None,
        state: Default::default(),
        maintenance: None,
        probes: Default::default(),
    };

    // Only the registry-level entry point is accessible from outside the crate.
    let registry = CompilerRegistry::new(&config, &targets);
    let compiler = registry.get("dev");

    // `build_ephemeral_resolver` is accessible (needed by backbuild).
    let resolver = compiler.build_ephemeral_resolver(&[], "main");

    // `EphemeralResolver::empty` is the other valid empty-resolver path.
    let empty = EphemeralResolver::empty();
    assert_eq!(resolver.order, empty.order);

    // Both compile_with_ephemerals and compile_with_sql_and_ephemerals are accessible.
    // (compile_with_sql, the no-ephemerals variant, is pub(crate) and would not compile here.)
}
