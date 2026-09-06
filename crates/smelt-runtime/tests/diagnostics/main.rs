//! TDD coverage for the shared `smelt-runtime::diagnostics` builder
//! (`docs/plans/20260725-ui-model-diagnostics.md` Phases 2a/2b;
//! `docs/specs/ui_model_diagnostics.md` §Surface "smelt-runtime builder";
//! §Semantics "Technique preview set" / "Admissibility verdict").
//!
//! These tests exercise `build_model_diagnostics`/`build_relation_contract`/
//! `build_plan_cell_diagnostics` against the real `examples/timeseries/`
//! fixture project — no synthetic in-memory model, except where a test
//! needs a specific `RowIdentity`/`Technique` shape a constructed
//! `PlanCell` demonstrates more directly than hunting a fixture for it —
//! per the plan's "real-fixture tests" convention.
//!
//! [`properties`] covers the `PropertySet`/relation-contract derivation;
//! [`preview`] and [`merge_preview`] cover the technique preview set,
//! split by fixture family (synthetic-cell previews vs. the
//! column-scoped-merge probe family).

use smelt_core::config::{
    CellTechnique, Config, Grain as ConfigGrain, Granularity, MaintenanceCellConfig,
    MaintenanceConfig, Materialization, RefreshStrategy, TimeseriesConfig,
};
use smelt_core::graph::DependencyGraph;
use smelt_core::metadata::ModelMetadata;
use smelt_core::workspace::load_workspace;
use smelt_core::{ModelFile, RefInfo, SmeltRef, SourceInfo};
use smelt_logical::analysis::source_bounds::BoundContext;
use smelt_logical::analysis::walk::Comparability;
use smelt_logical::maintenance::emit::MaintenanceDialect;
use smelt_logical::maintenance::{
    ColumnGroup, Corner, PartitionLocal, PlanCell, RowIdentity, RowIdentityVerdict, Technique,
    Trigger,
};
use smelt_runtime::diagnostics::{
    build_model_diagnostics, build_plan_cell_diagnostics, build_relation_contract, Admissibility,
};
use smelt_runtime::{build_source_timeseries_map, CompilerRegistry, EphemeralResolver};
use std::path::PathBuf;

fn timeseries_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries fixture must exist")
}

/// Load the real `examples/timeseries/` project's models, declared
/// sources, and `smelt.yml` config — the same calls `smelt explain` makes
/// (`crates/smelt-cli/src/commands/explain.rs`), but without any Salsa
/// database: `load_workspace` + `discover_source_infos` are both pure,
/// disk-reading functions.
fn load_fixture() -> (Vec<ModelFile>, Vec<SourceInfo>, Config) {
    let root = timeseries_root();
    let loaded = load_workspace(&root);
    assert!(
        loaded.errors.is_empty(),
        "examples/timeseries must load without errors: {:?}",
        loaded.errors
    );
    let source_infos = smelt_core::discover_source_infos(&root, &loaded.config.paths);
    (loaded.sql_files, source_infos, loaded.config)
}

fn find_model<'a>(models: &'a [ModelFile], name: &str) -> &'a ModelFile {
    models
        .iter()
        .find(|m| m.canonical_path() == name)
        .unwrap_or_else(|| panic!("model `{name}` not found in examples/timeseries"))
}

/// Derive `model`'s real `MaintenancePlan::cells` via the same plain
/// (non-Salsa) entry point `smelt-db`'s own Salsa wrapper calls
/// (`smelt_db::lib::maintenance_plan_report`'s doc comment: "gathers
/// `file`'s referenced sources … then calls `derive_maintenance_plan`") —
/// mirroring its input assembly without a Salsa database, matching this
/// crate's existing Salsa-purity-respecting pattern of taking
/// already-resolved facts. `allow_full_scan: true` for every referenced
/// source keeps this a pure "what would the plan admit" helper: it is not
/// re-testing the K8 partition-locality guardrail, only assembling cells
/// for the technique-preview builder to render.
fn derive_plan_cells(
    model: &ModelFile,
    source_infos: &[SourceInfo],
) -> (Vec<PlanCell>, Vec<ColumnGroup>) {
    let metadata = model
        .metadata
        .as_deref()
        .expect("fixture maintained model must declare frontmatter");
    let stripped_sql = smelt_parser::strip_frontmatter(&model.content).to_string();
    let refs = smelt_logical::collect_path_refs(&stripped_sql);
    let sources: Vec<smelt_logical::maintenance::SourceFacts> = refs
        .iter()
        .filter_map(|r| {
            let stripped = r.strip_prefix("smelt.")?;
            let bare = stripped.strip_prefix("sources.").unwrap_or(stripped);
            let segs: Vec<String> = stripped.split('.').map(|s| s.to_string()).collect();
            let info = source_infos.iter().find(|s| s.address_segments == segs);
            Some(smelt_db::queries::maintenance::source_facts(
                bare, info, true,
            ))
        })
        .collect();
    let table = model
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    smelt_db::queries::maintenance::derive_model_maintenance_plan(
        &stripped_sql,
        &table,
        metadata,
        &sources,
        &std::collections::HashSet::new(),
        None,
        &[],
        &[],
        &std::collections::BTreeMap::new(),
        None,
        None,
        &[],
    )
    .map(|r| (r.plan.cells, r.column_groups))
    .unwrap_or_default()
}

/// The compilation machinery a technique preview's illustrative SQL is
/// built through — `smelt-cli::explain`'s own `--show-sql` setup, minus
/// anything `--period`-specific (the technique-preview builder always
/// renders symbolic placeholders, `crates/smelt-runtime/src/diagnostics.rs`
/// `build_technique_statements`'s own doc comment).
struct CompileFixture {
    registry: CompilerRegistry,
    resolver: EphemeralResolver,
}

fn compile_fixture(config: &Config) -> CompileFixture {
    let registry = CompilerRegistry::new(config, &config.targets);
    let resolver = registry
        .get("dev")
        .build_ephemeral_resolver(&[], "main")
        .expect("no ephemerals");
    CompileFixture { registry, resolver }
}

/// A `BoundContext` naming every `smelt.sources.*` ref `model` reads, keyed
/// the same way `resolve_table_ref_source_name`/`RefInfo::to_path` name a
/// `smelt.sources.raw.transactions`-style ref: the dot-joined path segments
/// after `smelt.` (`"sources.raw.transactions"`), matching
/// `build_relation_contract`'s own source-edge lookup
/// (`crates/smelt-runtime/src/diagnostics.rs`).
fn bound_ctx_for(model: &ModelFile, source_infos: &[SourceInfo]) -> BoundContext {
    let mut ctx = BoundContext::new();
    for r in &model.refs {
        let segs = r.smelt_ref.to_path();
        if segs.first().map(String::as_str) != Some("sources") {
            continue;
        }
        let Some(info) = source_infos.iter().find(|s| s.address_segments == segs) else {
            continue;
        };
        if let Some(ts) = &info.timeseries {
            ctx.add_source(&segs.join("."), &ts.partition_column);
        }
    }
    ctx
}

mod merge_preview;
mod preview;
mod properties;
