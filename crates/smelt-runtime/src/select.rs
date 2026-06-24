//! Selection / filter pass.
//!
//! Resolve the user-supplied selectors and excludes against the project's
//! `DependencyGraph`, then drop entries that are never executed (test models,
//! generator files), and finally compute per-model target assignments plus
//! the cross-engine reference edges.
//!
//! Both `smelt-cli`'s `commands/run.rs` and `smelt-ui`'s `run_manager.rs` /
//! `build.rs` consume this function — the Run Pipeline Parity Rule's
//! "selection" half. Pure (graph-driven, no Salsa, no filesystem).
//!
//! Generator-emitted models are expected to already be present in the input
//! `DependencyGraph` — the graph-build step is responsible for expanding
//! `*.gen.sql` files via the Salsa `emitted_models` pipeline before passing
//! the graph here. This function only does selection over the graph it
//! receives.

use anyhow::Result;
use smelt_core::config::Config;
use smelt_core::graph::DependencyGraph;
use smelt_core::parse_selector;
use std::collections::HashMap;

/// Plan for which models execute and how their backends connect.
#[derive(Debug, Clone)]
pub struct SelectionPlan {
    /// Executable model names in topological order (tests and generator
    /// files already filtered out).
    pub ordered_models: Vec<String>,
    /// Per-model target name (from `smelt.yml` `target:` overrides or the
    /// request's default).
    pub target_assignments: HashMap<String, String>,
    /// Edges where a model and its dependency live on different backends —
    /// these become Parquet exchanges at execution time.
    /// Tuple is `(model_name, dep_name, model_target, dep_target)`.
    pub cross_engine_edges: Vec<(String, String, String, String)>,
}

/// Inputs for [`select_executable_models`]. A lightweight value rather than
/// the full `ExecuteRequest` so the function is callable from contexts
/// (preview, validate) that have not finalised every run field.
#[derive(Debug, Clone, Default)]
pub struct SelectionRequest {
    pub select: Vec<String>,
    pub exclude: Vec<String>,
    /// Default target name; per-model `target:` metadata overrides win.
    pub target: String,
}

/// Resolve selectors and excludes, drop tests and generator files, and
/// compute per-model targets plus cross-engine edges.
///
/// **Filter contract (the Run Pipeline Parity Rule's invariant):**
/// - Test models (`materialization: test`) are never returned. This filter
///   used to live independently in `smelt-cli/src/commands/run.rs` and in
///   the UI's `run_manager.rs` / `build.rs`; today's UI test-model panic
///   was an instance of the two filters drifting apart.
/// - Generator files (`*.gen.sql` files, identified by `.gen` name suffix
///   or `.gen.` path component) are never returned. Their bodies are
///   meta-language expressions, not executable SQL; the models they emit
///   are expected to already be expanded into the graph as virtual nodes.
///
/// The function is pure and graph-driven. The caller is responsible for
/// producing a `DependencyGraph` that already has emitted models present
/// (CLI does this via `discover_emitted_model_files` before building the
/// graph). When a graph does not include emitted models, selection simply
/// returns whatever is in the graph; emitted-models expansion is a graph-
/// construction concern, not a selection concern.
pub fn select_executable_models(
    graph: &DependencyGraph,
    config: &Config,
    request: &SelectionRequest,
) -> Result<SelectionPlan> {
    let mut selected_set = if request.select.is_empty() {
        graph.all_model_names()
    } else {
        let selectors: Vec<_> = request
            .select
            .iter()
            .map(|s| {
                parse_selector(s).map_err(|e| anyhow::anyhow!("Invalid selector '{}': {}", s, e))
            })
            .collect::<Result<_, _>>()?;
        graph.select_models(&selectors, config)?
    };

    if !request.exclude.is_empty() {
        let excludes: Vec<_> = request
            .exclude
            .iter()
            .map(|s| {
                parse_selector(s).map_err(|e| anyhow::anyhow!("Invalid exclude '{}': {}", s, e))
            })
            .collect::<Result<_, _>>()?;
        selected_set = graph.exclude_models(&selected_set, &excludes, config)?;

        // D-39: refuse an inconsistent working set — a retained model must not
        // have a direct built-model dependency that was removed by --exclude.
        let violations = graph.check_working_set_consistency(&selected_set);
        if !violations.is_empty() {
            let lines: Vec<String> = violations
                .iter()
                .map(|(retained, missing)| {
                    format!(
                        "  model '{}' requires upstream '{}' which was excluded",
                        retained, missing
                    )
                })
                .collect();
            return Err(anyhow::anyhow!(
                "Inconsistent working set after --exclude: the following retained models \
                 have missing upstream dependencies:\n{}",
                lines.join("\n")
            ));
        }
    }

    let ordered_models: Vec<String> = graph
        .filtered_execution_order(&selected_set)?
        .into_iter()
        .filter(|name| {
            let Ok(model) = graph.get_model(name) else {
                // Unknown entries pass through; downstream catches them.
                return true;
            };
            // Drop tests: they are never executed by `run`.
            if model.is_test() {
                return false;
            }
            // Drop generator files: their bodies are meta-language, not SQL.
            if is_generator_file(&model.name, &model.path.to_string_lossy()) {
                return false;
            }
            // Drop ephemeral models: they are inlined as CTEs into downstream
            // models, never materialized directly. Their SQL is absorbed by
            // `build_ephemeral_resolver` during compile-context construction.
            let mat = config.get_materialization_with_metadata(
                name,
                model.metadata.as_ref().map(|b| b.as_ref()),
            );
            if mat == smelt_core::config::Materialization::Ephemeral {
                return false;
            }
            true
        })
        .collect();

    let mut target_assignments: HashMap<String, String> = HashMap::new();
    for model_name in &ordered_models {
        let model = graph.get_model(model_name)?;
        let target = config.get_target(model_name, model.metadata.as_deref(), &request.target);
        target_assignments.insert(model_name.clone(), target);
    }

    let cross_engine_edges = graph.find_cross_backend_edges(&target_assignments);

    Ok(SelectionPlan {
        ordered_models,
        target_assignments,
        cross_engine_edges,
    })
}

/// Identify a generator file (`*.gen.sql`).
///
/// Matches what `smelt-cli/src/commands/run.rs` historically filtered:
/// either the model name ends with `.gen` (frontmatter-named generator)
/// or the path ends with `.gen.sql` (filename-based detection).
///
/// NOTE: emitted model virtual paths look like `…/foo.gen.sql::cohorts.us_west`.
/// These must NOT be treated as generator files — they are valid executable
/// models whose paths happen to reference the generator that produced them.
/// We match only paths that END with `.gen.sql` (the raw generator file),
/// not paths that merely CONTAIN `.gen.` (which would also match emitted
/// model virtual paths).
fn is_generator_file(name: &str, path: &str) -> bool {
    name.ends_with(".gen") || path.ends_with(".gen.sql")
}
