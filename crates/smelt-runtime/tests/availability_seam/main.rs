//! Phase 5 (`docs/outcomes/20260904-state-residency/outcome.md`) — the
//! single `smelt-runtime` derivation seam: every runtime consumer of
//! `smelt-db`'s `derive_model_maintenance_plan{,_with_edges}` reads an
//! availability-resolved plan via `smelt_runtime::maintenance_availability`,
//! never the bare `smelt-db` functions.
//!
//! Spec: `docs/specs/state.md` §"The degradation contract".
//!
//! [`structural`] holds the two source-scanning structural assertions;
//! this file holds the fixtures and the pure-behavior unit tests.

use std::collections::HashSet;

use smelt_core::config::{Config, Grain as ConfigGrain, RefreshStrategy, WarehouseTables};
use smelt_core::ModelMetadata;
use smelt_dialect::SqlDialect;
use smelt_logical::maintenance::availability::{
    realisable_state_structures, StateAvailability, StateStructure,
};
use smelt_logical::maintenance::derive::SourceReferentialIntegrity;
use smelt_logical::maintenance::{MutationProfile, SourceFacts, Technique, Trigger};
use smelt_runtime::maintenance_availability::{
    availability_for_run, derive_resolved, derive_resolved_with_edges,
};

mod builders;
mod structural;

/// A `grain: key` model whose driving source (`payments`) is append-only
/// with an invertible `SUM` combiner — the same shape
/// `crates/smelt-logical/tests/maintenance_availability.rs::keyed_fold_plan`
/// exercises at the pure-derivation layer, replicated here at the
/// `smelt-db` entry point so the seam is proven against a real
/// `Technique::KeyedFold` admission, not a hand-built cell.
const KEYED_FOLD_SQL: &str = "SELECT user_id, SUM(amount) AS lifetime_spend \
     FROM smelt.sources.payments GROUP BY user_id";

fn keyed_fold_metadata() -> ModelMetadata {
    ModelMetadata {
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(ConfigGrain::Key),
        ..Default::default()
    }
}

fn keyed_fold_sources() -> Vec<SourceFacts> {
    vec![SourceFacts {
        name: "payments".to_string(),
        mutation: MutationProfile::AppendOnly,
        partition_col: Some("pay_date".to_string()),
        unique_key: vec![],
        allow_full_scan: false,
    }]
}

fn creation_cell(
    plan: &smelt_logical::maintenance::MaintenancePlan,
) -> &smelt_logical::maintenance::PlanCell {
    plan.cells
        .iter()
        .find(|c| matches!(c.trigger, Trigger::NewData { .. }))
        .expect("the keyed-fold fixture must derive a creation cell")
}

#[test]
fn availability_for_run_intersects_dialect_and_warehouse_tables() {
    let (mut config, _) =
        Config::parse_with_warnings("name: p\nversion: 1\n").expect("minimal config must parse");
    assert_eq!(config.state.warehouse_tables, WarehouseTables::Allowed);

    let duckdb_allowed = availability_for_run(SqlDialect::DuckDB, &config);
    assert!(duckdb_allowed.contains(StateStructure::ReconciliationLedger));
    assert!(duckdb_allowed.contains(StateStructure::MergeLedger));

    config.state.warehouse_tables = WarehouseTables::None;
    let duckdb_none = availability_for_run(SqlDialect::DuckDB, &config);
    assert!(!duckdb_none.contains(StateStructure::ReconciliationLedger));
    assert!(!duckdb_none.contains(StateStructure::MergeLedger));

    // The dialect leg of the intersection, shown on a dialect that realises
    // nothing: `warehouse_tables: allowed` cannot conjure a structure the
    // backend has no builder for. (Before 2026-09-10 this asserted Spark
    // realised the fingerprint sidecar — it does not, and that false claim
    // is what suppressed the downgrade the T5 run path needed; see
    // `docs/outcomes/20260906-bigquery-correctness` decision log.)
    config.state.warehouse_tables = WarehouseTables::Allowed;
    let spark_allowed = availability_for_run(SqlDialect::SparkSQL, &config);
    assert!(!spark_allowed.contains(StateStructure::ReconciliationLedger));
    assert!(!spark_allowed.contains(StateStructure::FingerprintSidecar));
    assert!(!spark_allowed.contains(StateStructure::ObservedOutputDeltas));
    // …while the same `allowed` config over DuckDB does realise them, so the
    // assertion above is about the dialect and not about `warehouse_tables`.
    assert!(availability_for_run(SqlDialect::DuckDB, &config)
        .contains(StateStructure::FingerprintSidecar));
}

/// A keyed-fold cell derived through [`derive_resolved`] under a
/// ledger-less availability downgrades to `PerGroupRecompute` and carries
/// the recorded `state_downgrade` — the seam actually applies
/// `resolve_availability`, not just re-deriving the ideal plan.
#[test]
fn derive_resolved_downgrades_a_keyed_fold_cell() {
    let metadata = keyed_fold_metadata();
    let sources = keyed_fold_sources();
    let ledger_less = StateAvailability::resolve(
        WarehouseTables::Allowed,
        &realisable_state_structures(SqlDialect::SparkSQL),
    );

    let result = derive_resolved(
        KEYED_FOLD_SQL,
        "main.lifetime_spend",
        &metadata,
        &sources,
        &HashSet::new(),
        None,
        &[],
        &[],
        &SourceReferentialIntegrity::new(),
        None,
        None,
        &ledger_less,
        &[],
    )
    .expect("a keyed-fold model must derive a plan");

    let cell = creation_cell(&result.plan);
    assert_eq!(cell.technique, Technique::PerGroupRecompute);
    let downgrade = cell
        .state_downgrade
        .as_ref()
        .expect("a ledger-less target must record the downgrade");
    assert_eq!(downgrade.original, Technique::KeyedFold);
    assert_eq!(downgrade.missing, StateStructure::ReconciliationLedger);
}

/// The edge-aware entry point applies the SAME resolution — proven here
/// with empty `model_edges` (the with-edges wrapper is a strict superset of
/// the source-only derivation; a real live `ColumnScopedMerge`/`MergeLedger`
/// admission needs dimension-join structural machinery this seam test does
/// not attempt to stand up, but `resolve_availability` treats every
/// ledger-requiring technique identically — proven exhaustively at the pure
/// layer by `crates/smelt-logical/tests/maintenance_availability.rs`).
#[test]
fn derive_resolved_with_edges_downgrades_the_same_keyed_fold_cell() {
    let metadata = keyed_fold_metadata();
    let sources = keyed_fold_sources();
    let ledger_less = StateAvailability::resolve(
        WarehouseTables::Allowed,
        &realisable_state_structures(SqlDialect::SparkSQL),
    );

    let result = derive_resolved_with_edges(
        KEYED_FOLD_SQL,
        "main.lifetime_spend",
        &metadata,
        &sources,
        &HashSet::new(),
        &[],
        None,
        &[],
        &[],
        &SourceReferentialIntegrity::new(),
        None,
        None,
        &ledger_less,
        &[],
    )
    .expect("a keyed-fold model must derive a plan through the edge-aware entry point too");

    let cell = creation_cell(&result.plan);
    assert_eq!(cell.technique, Technique::PerGroupRecompute);
    assert!(cell.state_downgrade.is_some());
}

/// Resolution is a no-op under full availability — a DuckDB target with
/// `warehouse_tables: allowed` sees byte-identical cells to the raw
/// `smelt-db` derivation, so this phase changes no behaviour on the
/// backend every existing fixture assumes.
#[test]
fn derive_resolved_under_full_availability_is_byte_identical_to_the_raw_derivation() {
    let metadata = keyed_fold_metadata();
    let sources = keyed_fold_sources();

    let raw = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        KEYED_FOLD_SQL,
        "main.lifetime_spend",
        &metadata,
        &sources,
        &HashSet::new(),
        None,
        &[],
        &[],
        &SourceReferentialIntegrity::new(),
        None,
        None,
        &[],
    )
    .expect("raw derivation must succeed");

    let resolved = derive_resolved(
        KEYED_FOLD_SQL,
        "main.lifetime_spend",
        &metadata,
        &sources,
        &HashSet::new(),
        None,
        &[],
        &[],
        &SourceReferentialIntegrity::new(),
        None,
        None,
        &StateAvailability::all(),
        &[],
    )
    .expect("resolved derivation must succeed");

    assert_eq!(raw.plan.cells.len(), resolved.plan.cells.len());
    for (raw_cell, resolved_cell) in raw.plan.cells.iter().zip(resolved.plan.cells.iter()) {
        assert_eq!(raw_cell.technique, resolved_cell.technique);
        assert!(raw_cell.state_downgrade.is_none());
        assert!(resolved_cell.state_downgrade.is_none());
    }
}

/// Phase 6c (`docs/outcomes/20260913-trino-incremental/phases/06c-plan.md`)
/// test 6: for a structure-less project, `resolve_repair_state_downgrade`
/// reports the repair-admitted cell's downgrade, and
/// `resolve_live_per_group_recompute_cell` returns `None` for the same cell
/// — no double dispatch of one cell through two different execution routes.
/// The fixture mirrors `repair_lowering.rs`'s own `REPAIR_MODEL_SQL`: `MAX`
/// (non-invertible) folded per `customer_id` over a clocked
/// `mutable_snapshot` source, with an explicit Form B band on `order_date`
/// that discharges obligation 4 (bounded per-group read footprint).
#[test]
fn repair_downgrade_routes_to_whole_target_rebuild() {
    const REPAIR_MODEL_SQL: &str = "SELECT customer_id, MAX(amount) AS max_amount \
         FROM smelt.sources.raw.orders \
         WHERE order_date BETWEEN TIMESTAMP '2025-01-12' - INTERVAL '1 day' AND TIMESTAMP \
         '2025-01-12' \
         GROUP BY customer_id";
    let smelt_core::FileMetadata::Single { metadata, .. } = smelt_core::extract_file_metadata(
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\nunique_key: \
         customer_id\n---\n",
    )
    .expect("parse frontmatter") else {
        panic!("single-model file");
    };
    let sources = vec![SourceFacts {
        name: "raw.orders".to_string(),
        mutation: MutationProfile::MutableSnapshot,
        partition_col: Some("order_date".to_string()),
        unique_key: vec!["order_id".to_string()],
        allow_full_scan: false,
    }];
    let mut explicitly_mutable = HashSet::new();
    explicitly_mutable.insert("raw.orders".to_string());

    let structure_less = StateAvailability::none();

    let downgrade = smelt_runtime::maintenance_driver::resolve_repair_state_downgrade(
        REPAIR_MODEL_SQL,
        "customer_max_amount",
        &metadata,
        &sources,
        &explicitly_mutable,
        &structure_less,
    )
    .expect("a repair-admitted cell must report its downgrade on a structure-less backend");
    assert_eq!(downgrade.original, Technique::PerGroupRecompute);
    assert_eq!(downgrade.missing, StateStructure::FingerprintSidecar);

    let live_cell = smelt_runtime::maintenance_driver::resolve_live_per_group_recompute_cell(
        REPAIR_MODEL_SQL,
        "customer_max_amount",
        &metadata,
        &sources,
        &explicitly_mutable,
        &[],
        SqlDialect::DuckDB,
        false,
        &structure_less,
    )
    .expect("resolver must not error");
    assert!(
        live_cell.is_none(),
        "a repair-admitted cell reached by the downgrade must not ALSO resolve live through \
         the repair-family driver — that would dispatch the same cell twice"
    );
}

/// `docs/outcomes/20260913-trino-ledger/outcome.md` phase 4, criterion 4: a
/// model targeting a `trino` backend (no `MaintenanceDialect` mapping) must
/// still be profiled through `smelt_runtime::profile::profiles_for_workspace`
/// — never dropped into `WorkspaceProfiles::failures` the way it was before
/// this phase (`profile.rs`'s old `Err(e) => { out.failures.insert(...);
/// continue; }` on `smelt_backend::maintenance_dialect`). Its cells must
/// carry the `state_downgrade` Trino's ledger-less realisable set forces,
/// exactly like the Spark-targeted model in `tests/fixtures/
/// dual_target_dialect` already proves for a different ledger-less dialect.
#[test]
fn a_trino_target_model_is_profiled_not_failed() {
    let project_dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dual_target_dialect");
    let loaded = smelt_core::workspace::load_workspace(&project_dir);
    let result = smelt_runtime::profile::profiles_for_workspace(&loaded)
        .expect("profiles_for_workspace must not fail on the dual-target fixture");

    assert!(
        !result.failures.contains_key("lifetime_spend_trino"),
        "a trino-targeted model must not be dropped into `failures`: {:?}",
        result.failures.get("lifetime_spend_trino")
    );
    let trino_profile = result
        .profiles
        .get("lifetime_spend_trino")
        .expect("lifetime_spend_trino must have a derived profile");
    assert!(
        trino_profile
            .cell_verdicts
            .iter()
            .any(|c| c.state_downgrade.is_some()),
        "the Trino-targeted model must show a state_downgrade (no ledger builder on \
         Trino): {:?}",
        trino_profile.cell_verdicts
    );
}

/// `docs/outcomes/20260913-trino-incremental/phases/06f-plan.md` test 5: the
/// double-dispatch shape phase 6d fixed for membership recompute
/// (`execute/project/mod.rs`'s `used_membership_recompute` arm gained `&&
/// whole_target_rebuild_downgrade.is_none()`) has a structural LOOKALIKE at
/// the column-scoped-merge dispatch site (~line 2216), which carries no such
/// guard. **Measured here: it is not the same bug — the two live cells can
/// never coexist for one model, so there is nothing for that site to
/// double-dispatch.**
///
/// `resolve_keyed_fold_state_downgrade`/`resolve_repair_state_downgrade`
/// (the two `whole_target_rebuild_downgrade` sources) only ever fire for a
/// `Grain::Key` model's OWN `NewData` fold cell (`Technique::KeyedFold` or a
/// repair-admitted `PerGroupRecompute`), which requires that model's
/// top-level `SELECT` to carry a `GROUP BY` (`derive_new_data`'s fold
/// admission). But `resolve_live_column_scoped_cell` only resolves
/// `Technique::ColumnScopedMerge` for a source whose column group has EMPTY
/// `membership_sensitivity` (`derive::mutation::derive_mutation`), and
/// `skeleton_source_closure`'s v1 scope restriction
/// (`crates/smelt-logical/src/analysis/skeleton_closure.rs` line ~131:
/// "this scope has a GROUP BY above the enrichment join ... is Open
/// regardless of the five conjuncts") makes that proof impossible whenever
/// the enrichment join shares a scope with a `GROUP BY` — which it always
/// does here, since the fold's own `GROUP BY` and any enrichment join
/// reachable from `column_scoped_cell` are the SAME top-level scope. An
/// unpruned (`Open`) enrichment source always lands in
/// `membership_sensitivity` (`grouping.rs::scan_scope_membership`'s
/// unconditional `sensitivity.insert`), forcing `Technique::DeleteInsert`,
/// never `ColumnScopedMerge`, for that source.
///
/// The other route to a live `ColumnScopedMerge` cell — a model-edge
/// enrichment (`append_model_edge_cells`,
/// `crates/smelt-logical/tests/model_edge_enrichment_mutation.rs`) — is
/// documented there as reachable only for "a `grain: partition` downstream
/// with no derivable key grain"; a `grain: partition` model's OWN creation
/// technique is always `Technique::DeleteInsert` (reachable, never
/// downgraded — `docs/specs/multi_backend.md` §"Incremental & schema
/// evolution per backend"), so `whole_target_rebuild_downgrade` is never
/// derived for it either.
///
/// So: a live `ColumnScopedMerge` cell needs a scope with no governing
/// `GROUP BY`; a live whole-target-rebuild downgrade needs one. Proven here
/// with the exact fixture
/// `crates/smelt-logical/tests/maintenance_new_data_enrich_only_waiver.rs::
/// admits_enrich_only_covered_mutable_source` uses at the hand-built-
/// `ColumnGroup` layer (which bypasses the real grouping walk and so cannot
/// see this restriction) — over the REAL SQL-derived walk, `dim`'s cell
/// lands on `Technique::DeleteInsert`, never `ColumnScopedMerge`, so
/// `resolve_live_column_scoped_cell` correctly returns `None` and the
/// `execute/project/mod.rs` dispatch site this test targets is
/// unreachable-together with a whole-target rebuild today. No fix is made;
/// this test is the regression guard should a future classifier change
/// (e.g. relaxing the v1 scope restriction) make the combination reachable.
#[test]
fn column_scoped_cell_is_suppressed_by_a_whole_target_rebuild() {
    let sql = "SELECT f.user_id AS user_id, SUM(f.amount) AS lifetime_spend, \
               d.status AS status \
               FROM smelt.sources.fact f \
               LEFT JOIN smelt.sources.dim d ON f.user_id = d.user_id \
               GROUP BY f.user_id";
    let metadata = ModelMetadata {
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(ConfigGrain::Key),
        unique_key: Some(vec!["user_id".to_string()]),
        ..Default::default()
    };
    let sources = vec![
        SourceFacts {
            name: "fact".to_string(),
            mutation: MutationProfile::AppendOnly,
            partition_col: None,
            unique_key: vec![],
            allow_full_scan: false,
        },
        SourceFacts {
            name: "dim".to_string(),
            mutation: MutationProfile::MutableSnapshot,
            partition_col: None,
            unique_key: vec!["user_id".to_string()],
            allow_full_scan: true,
        },
    ];
    let explicitly_mutable: HashSet<String> = ["dim".to_string()].into_iter().collect();
    let ledger_less = StateAvailability::resolve(
        WarehouseTables::Allowed,
        &realisable_state_structures(SqlDialect::SparkSQL),
    );

    let downgrade = smelt_runtime::maintenance_driver::resolve_keyed_fold_state_downgrade(
        sql,
        "user_lifetime_status",
        &metadata,
        &sources,
        &explicitly_mutable,
        &ledger_less,
    );
    assert!(
        downgrade.is_some(),
        "fact's additive fold must downgrade to a whole-target rebuild on a ledger-less target"
    );

    let column_scoped = smelt_runtime::maintenance_driver::resolve_live_column_scoped_cell(
        sql,
        "user_lifetime_status",
        &metadata,
        &sources,
        &explicitly_mutable,
        &[],
        true,
        &[],
        &ledger_less,
    )
    .expect("resolve_live_column_scoped_cell must not error");

    assert!(
        column_scoped.is_none(),
        "measured unreachable: dim's enrichment join shares a scope with fact's fold GROUP BY, \
         so skeleton_source_closure's v1 scope restriction can never prune its membership \
         sensitivity — dim's cell must land on Technique::DeleteInsert, never ColumnScopedMerge, \
         got {column_scoped:?}"
    );
}
