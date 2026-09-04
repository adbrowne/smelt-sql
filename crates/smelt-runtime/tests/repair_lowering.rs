//! Runtime lowering for the repair family (`docs/specs/incremental_models.md`
//! §"The repair family"): an admitted `Technique::PerGroupRecompute` cell
//! resolves live off the derived maintenance plan, its two emitter string
//! inputs are built by pure builders, and the resulting `StatementGroup`
//! repairs exactly the affected key groups.
//!
//! The unit legs (1–4) need no backend; the DuckDB legs (5–6) drive a real
//! `execute_project` run so the repair is proven against stored state, not
//! against a hand-assembled statement list.

use std::collections::HashSet;

use smelt_logical::analysis::source_bounds::Seconds;
use smelt_logical::maintenance::choice::ChosenTechnique;
use smelt_logical::maintenance::diff_patch::DeleteLeg;
use smelt_logical::maintenance::emit::Region;
use smelt_logical::maintenance::{
    MutationProfile, RowIdentity, RowIdentityVerdict, ScanClamp, SourceFacts, Technique,
};
use smelt_runtime::maintenance_driver::{
    repair_affected_keys_select, repair_candidate_select, resolve_live_per_group_recompute_cell,
    resolve_repair_write,
};

/// The keyed fixture this file's unit legs share: `MAX` (a non-invertible
/// combiner under retraction) folded per `customer_id` over `raw.orders`, a
/// **clocked** `mutable_snapshot` source. The explicit Form B band on
/// `order_date` is what discharges the repair family's obligation 4
/// (bounded per-group read footprint) — without it
/// `repair::admit_per_group_recompute` refuses `RepairSliceUnbounded`.
const REPAIR_MODEL_SQL: &str = "SELECT customer_id, MAX(amount) AS max_amount \
     FROM smelt.sources.raw.orders \
     WHERE order_date BETWEEN TIMESTAMP '2025-01-12' - INTERVAL '1 day' AND TIMESTAMP \
     '2025-01-12' \
     GROUP BY customer_id";

const REPAIR_MODEL_FILE: &str = "---\n\
     materialization: table\n\
     refresh: incremental\n\
     grain: key\n\
     unique_key: customer_id\n\
     ---\n";

/// An ordinary append-only keyed fold — the shape the repair narrowing must
/// leave alone (the faithful-fold source-posture obligation passes, so a
/// `Technique::KeyedFold` cell is admitted and no repair cell exists).
const APPEND_ONLY_MODEL_SQL: &str = "SELECT customer_id, MAX(amount) AS max_amount \
     FROM smelt.sources.raw.orders \
     WHERE order_date BETWEEN TIMESTAMP '2025-01-12' - INTERVAL '1 day' AND TIMESTAMP \
     '2025-01-12' \
     GROUP BY customer_id";

fn metadata_and_sql(text: &str) -> (smelt_core::ModelMetadata, String) {
    let smelt_core::FileMetadata::Single {
        metadata,
        sql_offset,
    } = smelt_core::extract_file_metadata(text).expect("parse frontmatter")
    else {
        panic!("single-model file");
    };
    (*metadata, text[sql_offset..].to_string())
}

fn orders_source(mutation: MutationProfile) -> SourceFacts {
    SourceFacts {
        name: "raw.orders".to_string(),
        mutation,
        partition_col: Some("order_date".to_string()),
        unique_key: vec!["order_id".to_string()],
        allow_full_scan: false,
    }
}

fn mutable_orders() -> (Vec<SourceFacts>, HashSet<String>) {
    let mut explicitly_mutable = HashSet::new();
    explicitly_mutable.insert("raw.orders".to_string());
    (
        vec![orders_source(MutationProfile::MutableSnapshot)],
        explicitly_mutable,
    )
}

// ── 1 ────────────────────────────────────────────────────────────────────
#[test]
fn resolve_live_per_group_recompute_cell_finds_the_admitted_repair_cell() {
    let text = format!("{REPAIR_MODEL_FILE}{REPAIR_MODEL_SQL}\n");
    let (metadata, sql) = metadata_and_sql(&text);
    let (sources, explicitly_mutable) = mutable_orders();

    let (source, cell, key, slice, write, _discovery) = resolve_live_per_group_recompute_cell(
        &sql,
        "customer_max_amount",
        &metadata,
        &sources,
        &explicitly_mutable,
        &[],
        smelt_dialect::SqlDialect::DuckDB,
    )
    .expect("resolver must not error")
    .expect("a live per-group-recompute cell must resolve for raw.orders");

    assert_eq!(source, "raw.orders");
    assert_eq!(cell.technique, Technique::PerGroupRecompute);
    // The key is the cell's own `RowIdentity::Key`, verbatim — never
    // re-derived from the model's declared `unique_key`.
    assert_eq!(
        cell.row_identity.identity,
        RowIdentity::Key(key.clone()),
        "the returned key must be the cell's own proven row identity"
    );
    assert_eq!(key, vec!["customer_id".to_string()]);
    assert_eq!(slice.source, "raw.orders");
    assert_eq!(slice.column, "order_date");
    assert_eq!(slice.before, Seconds::days(1));
    assert_eq!(
        write,
        smelt_runtime::maintenance_driver::RepairWrite::TargetedDeleteInsert,
        "no write pin, so the resolver must return the repair family's own targeted write"
    );
}

// ── P9 test 3 ────────────────────────────────────────────────────────────
/// A `MutationProfile::MutableSnapshot` delta posture routes discovery to
/// the group-grain sidecar diff (`RepairDiscovery::SidecarDiff`), never the
/// clamped `SELECT DISTINCT` (`docs/specs/incremental_models.md` §"The
/// repair family" — "Obligation 7 over a `mutable_snapshot` source"). The
/// append-only posture's own `ClampedScan` shape is unchanged — proven
/// directly by `affected_keys_select_bounds_the_read_with_the_cells_scan_clamp`
/// (test 4), since an append-only source never admits a repair cell at all
/// in this fixture (its fold succeeds, so there is nothing to narrow).
#[test]
fn snapshot_source_discovery_uses_the_sidecar_diff() {
    let text = format!("{REPAIR_MODEL_FILE}{REPAIR_MODEL_SQL}\n");
    let (metadata, sql) = metadata_and_sql(&text);
    let (sources, explicitly_mutable) = mutable_orders();

    let (_source, _cell, _key, _slice, _write, discovery) = resolve_live_per_group_recompute_cell(
        &sql,
        "customer_max_amount",
        &metadata,
        &sources,
        &explicitly_mutable,
        &[],
        smelt_dialect::SqlDialect::DuckDB,
    )
    .expect("resolver must not error")
    .expect("a live per-group-recompute cell must resolve for raw.orders");

    match discovery {
        smelt_runtime::maintenance_driver::RepairDiscovery::SidecarDiff { digest_columns } => {
            assert!(
                !digest_columns.is_empty(),
                "a MutableSnapshot source's discovery must carry non-empty P4-projection digest \
                 columns"
            );
        }
        other => panic!(
            "expected RepairDiscovery::SidecarDiff for a MutableSnapshot source, got {other:?}"
        ),
    }
}

// ── P9 test 4 ────────────────────────────────────────────────────────────
/// A non-DuckDB backend must fail loud rather than silently falling back to
/// the unsound current-source scan (mirrors
/// `diff_fingerprint_sidecar_changed_keys`'s own precedent).
#[test]
fn snapshot_discovery_fails_loud_on_a_non_duckdb_backend() {
    let text = format!("{REPAIR_MODEL_FILE}{REPAIR_MODEL_SQL}\n");
    let (metadata, sql) = metadata_and_sql(&text);
    let (sources, explicitly_mutable) = mutable_orders();

    let err = resolve_live_per_group_recompute_cell(
        &sql,
        "customer_max_amount",
        &metadata,
        &sources,
        &explicitly_mutable,
        &[],
        smelt_dialect::SqlDialect::SparkSQL,
    )
    .expect_err(
        "a MutableSnapshot source's sidecar-diff discovery must fail loud on a non-DuckDB \
         backend, never silently use the unsound current-source scan",
    );

    let backend_err = err
        .downcast_ref::<smelt_backend::BackendError>()
        .unwrap_or_else(|| panic!("expected a BackendError::unsupported, got: {err}"));
    assert!(
        matches!(
            backend_err,
            smelt_backend::BackendError::UnsupportedFeature { .. }
        ),
        "expected BackendError::UnsupportedFeature, got: {backend_err:?}"
    );
}

// ── 1b ───────────────────────────────────────────────────────────────────
#[test]
fn diff_patch_pin_over_a_repair_cell_resolves_a_diff_patch_write() {
    let text = format!(
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: key\n\
         unique_key: customer_id\n\
         maintenance:\n\
         \x20\x20cells:\n\
         \x20\x20- on: raw.orders\n\
         \x20\x20\x20\x20columns: [max_amount]\n\
         \x20\x20\x20\x20write: diff_patch\n\
         ---\n{REPAIR_MODEL_SQL}\n"
    );
    let (metadata, sql) = metadata_and_sql(&text);
    let (sources, explicitly_mutable) = mutable_orders();

    let (_source, _cell, _key, _slice, write, _discovery) = resolve_live_per_group_recompute_cell(
        &sql,
        "customer_max_amount",
        &metadata,
        &sources,
        &explicitly_mutable,
        &[],
        smelt_dialect::SqlDialect::DuckDB,
    )
    .expect("resolver must not error")
    .expect("a live per-group-recompute cell must resolve for raw.orders");

    match write {
        smelt_runtime::maintenance_driver::RepairWrite::DiffPatch {
            compared_columns,
            delete_leg,
        } => {
            assert!(
                !compared_columns.is_empty(),
                "diff_patch must resolve a non-empty compared-column set"
            );
            assert_eq!(
                delete_leg,
                smelt_logical::maintenance::diff_patch::DeleteLeg::Complete,
                "PerGroupRecompute's own bounded-slice admission discharges diff_patch's \
                 completeness premise"
            );
        }
        other => panic!("expected RepairWrite::DiffPatch, got {other:?}"),
    }
}

// ── 1c ───────────────────────────────────────────────────────────────────
/// `write: diff_patch` over the append-only model resolves no live repair
/// cell at all — the fold cell serves this trigger, and
/// `resolve_live_per_group_recompute_cell` only ever considers
/// `Technique::PerGroupRecompute` cells, so a pin that cannot apply to a
/// repair cell is simply not this resolver's concern (a `ChoiceRefusal` for
/// an unaddressable pin, if any, is the pre-execution diagnostic gate's job,
/// not this runtime resolver's).
#[test]
fn resolve_live_per_group_recompute_cell_ignores_a_pin_with_no_repair_cell_to_address() {
    let text = format!(
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: key\n\
         unique_key: customer_id\n\
         maintenance:\n\
         \x20\x20cells:\n\
         \x20\x20- on: raw.orders\n\
         \x20\x20\x20\x20columns: [max_amount]\n\
         \x20\x20\x20\x20write: diff_patch\n\
         ---\n{APPEND_ONLY_MODEL_SQL}\n"
    );
    let (metadata, sql) = metadata_and_sql(&text);
    let sources = vec![orders_source(MutationProfile::AppendOnly)];

    let resolved = resolve_live_per_group_recompute_cell(
        &sql,
        "customer_max_amount",
        &metadata,
        &sources,
        &HashSet::new(),
        &[],
        smelt_dialect::SqlDialect::DuckDB,
    )
    .expect("resolver must not error");
    assert!(
        resolved.is_none(),
        "an append-only model's trigger must never resolve a live repair cell, got {resolved:?}"
    );
}

// ── 1d ───────────────────────────────────────────────────────────────────
/// [`resolve_repair_write`]'s decision table, unit-tested directly (split
/// out of `resolve_live_per_group_recompute_cell`'s loop precisely so this
/// is reachable without a full plan derivation): a `diff_patch` pin whose
/// underlying recompute is NOT `PerGroupRecompute` — the region `DeleteInsert`
/// default — fails loud by name rather than falling through to the default
/// write; every other resolvable-but-not-this-cell `ChosenTechnique` (a
/// `technique: recompute` pin, or this cell simply not being the one that
/// admitted) returns `Ok(None)`, never an error.
#[test]
fn resolve_repair_write_decision_table() {
    let identity = RowIdentityVerdict {
        identity: RowIdentity::Key(vec!["customer_id".to_string()]),
        proven_mismatch: None,
    };
    let group_columns = vec!["max_amount".to_string()];

    let write = resolve_repair_write(
        &ChosenTechnique::Admitted(Technique::PerGroupRecompute),
        &group_columns,
        &[],
        &identity,
        "{max_amount}",
    )
    .expect("must not error")
    .expect("must resolve a write");
    assert_eq!(
        write,
        smelt_runtime::maintenance_driver::RepairWrite::TargetedDeleteInsert
    );

    let err = resolve_repair_write(
        &ChosenTechnique::DiffPatch {
            recompute: Technique::DeleteInsert,
            delete_leg: DeleteLeg::Omitted {
                why: "not proven complete".to_string(),
            },
        },
        &group_columns,
        &[],
        &identity,
        "{max_amount}",
    )
    .expect_err("a diff_patch pin over the region DeleteInsert default must fail loud");
    assert!(
        err.to_string().contains("MaintenanceDiffPatchUnroutable"),
        "{err}"
    );

    assert!(
        resolve_repair_write(
            &ChosenTechnique::RegionRecompute,
            &group_columns,
            &[],
            &identity,
            "{max_amount}",
        )
        .expect("must not error")
        .is_none(),
        "a technique: recompute pin is simply not this live cell — never an error"
    );
    assert!(
        resolve_repair_write(
            &ChosenTechnique::Admitted(Technique::ColumnScopedMerge),
            &group_columns,
            &[],
            &identity,
            "{max_amount}",
        )
        .expect("must not error")
        .is_none(),
        "an admitted technique other than PerGroupRecompute is not this live cell either"
    );
}

// ── 2 ────────────────────────────────────────────────────────────────────
#[test]
fn resolve_live_per_group_recompute_cell_none_for_an_append_only_model() {
    let text = format!("{REPAIR_MODEL_FILE}{APPEND_ONLY_MODEL_SQL}\n");
    let (metadata, sql) = metadata_and_sql(&text);
    let sources = vec![orders_source(MutationProfile::AppendOnly)];

    let resolved = resolve_live_per_group_recompute_cell(
        &sql,
        "customer_max_amount",
        &metadata,
        &sources,
        &HashSet::new(),
        &[],
        smelt_dialect::SqlDialect::DuckDB,
    )
    .expect("resolver must not error");

    assert!(
        resolved.is_none(),
        "repair never displaces an admitted fold cell, got {resolved:?}"
    );
}

// ── 3 ────────────────────────────────────────────────────────────────────
/// A `PerGroupRecompute` cell carrying `RowIdentity::WholeRow` is
/// structurally impossible today (`repair::derive_repair_cell` always builds
/// `RowIdentity::Key`), so this exercises the resolver's fail-loud leg
/// directly on a synthetic cell: it must error by name rather than silently
/// widening the repair to every stored row.
#[test]
fn resolve_live_per_group_recompute_cell_fails_loud_on_whole_row_identity() {
    use smelt_logical::maintenance::{
        Corner, PartitionLocal, PlanCell, RowIdentityVerdict, Trigger,
    };

    let cell = PlanCell {
        group: "{max_amount}".to_string(),
        trigger: Trigger::NewData {
            source: "raw.orders".to_string(),
        },
        corner: Corner::ColumnMerge,
        technique: Technique::PerGroupRecompute,
        partition_local: PartitionLocal::Yes,
        scans: vec![ScanClamp {
            source: "raw.orders".to_string(),
            column: "order_date".to_string(),
            before: Seconds::days(1),
            after: Seconds::ZERO,
            write_footprint: None,
        }],
        ledger_catch_up: false,
        row_identity: RowIdentityVerdict {
            identity: RowIdentity::WholeRow,
            proven_mismatch: None,
        },
        skeleton_source_closure: None,
        fingerprint_projections: std::collections::BTreeMap::new(),
        key_scope: None,
        state_downgrade: None,
    };

    let err = smelt_runtime::maintenance_driver::repair_cell_key(&cell)
        .expect_err("a WholeRow per-group-recompute cell must fail loud");
    let msg = err.to_string();
    assert!(
        msg.contains("RowIdentity::WholeRow") && msg.contains("PerGroupRecompute"),
        "the refusal must name the failing identity and technique, got: {msg}"
    );
}

// ── 4 ────────────────────────────────────────────────────────────────────
#[test]
fn affected_keys_select_bounds_the_read_with_the_cells_scan_clamp() {
    let key = vec!["customer_id".to_string()];
    let region = Region {
        start: "'2025-01-12'".to_string(),
        end: "'2025-01-13'".to_string(),
    };
    let clamp = ScanClamp {
        source: "raw.orders".to_string(),
        column: "order_date".to_string(),
        before: Seconds::days(1),
        after: Seconds::ZERO,
        write_footprint: None,
    };

    // The affected-key relation is a single canonical `delta_key` column
    // (P9, `docs/outcomes/20260809-repair-family/phases/09-plan.md` task 3)
    // — the SAME `key_expr_for_columns` shape `emit_fingerprint_digest_select`
    // uses, never raw key columns, since a deleted group's typed column
    // values are unrecoverable by construction.
    let delta_key_expr = "COALESCE(CAST(customer_id AS VARCHAR), '\u{2}NULL\u{2}')";

    let clamped =
        repair_affected_keys_select("main.sources_raw_orders", &key, Some(&clamp), &region);
    assert_eq!(
        clamped,
        format!(
            "SELECT DISTINCT {delta_key_expr} AS delta_key FROM main.sources_raw_orders WHERE \
             order_date >= '2025-01-12' - INTERVAL '86400 seconds' AND order_date < \
             '2025-01-13'"
        ),
        "the affected-keys read must carry the cell's own derived clamp predicate"
    );

    // Only reachable where admission already proved the slice by another
    // route — an unclamped scan is the unpredicated read, never a
    // silently-narrowed one.
    let unclamped = repair_affected_keys_select("main.sources_raw_orders", &key, None, &region);
    assert_eq!(
        unclamped,
        format!("SELECT DISTINCT {delta_key_expr} AS delta_key FROM main.sources_raw_orders")
    );

    // The candidate carries NO clamp of its own — the group is recomputed
    // whole, semi-joined to the affected keys.
    let candidate = repair_candidate_select("SELECT customer_id, 1 AS v", &key, &clamped);
    assert!(
        !candidate.contains("INTERVAL '86400 seconds' AND order_date <")
            || candidate.matches("86400 seconds").count() == 1,
        "the clamp belongs to the affected-keys read only, not the output wrapper: {candidate}"
    );
    assert!(
        candidate.contains("EXISTS") && candidate.contains("SELECT customer_id, 1 AS v"),
        "the candidate is the model's own full SQL semi-joined to the affected keys: {candidate}"
    );
    assert!(
        candidate.contains("__smelt_repair_keys.delta_key"),
        "the candidate joins the affected-keys relation by its canonical `delta_key` column: \
         {candidate}"
    );

    // The `diff_patch` slice restriction is the SAME affected-key read,
    // wrapped as an EXISTS predicate, table-qualified on every key column —
    // never a partition region (a keyed aggregate output has none).
    let slice_predicate = smelt_runtime::maintenance_driver::repair_slice_predicate(
        "main.customer_max_amount",
        &key,
        &clamped,
    );
    assert_eq!(
        slice_predicate,
        format!(
            "EXISTS (SELECT 1 FROM ({clamped}) AS __smelt_repair_keys WHERE \
             COALESCE(CAST(main.customer_max_amount.customer_id AS VARCHAR), \
             '\u{2}NULL\u{2}') = __smelt_repair_keys.delta_key)"
        )
    );
}

/// `repair_augmented_model_sql_appends_state_columns` (P10 test 2): widens
/// the model SQL with one `, <per_partition_expr> AS <name>` select item per
/// state column, and returns the SQL unchanged for an empty state-column
/// list; an unparseable body errors by name.
#[test]
fn repair_augmented_model_sql_appends_state_columns() {
    use smelt_logical::analysis::decomposed_state::StateColumn;
    use smelt_logical::rules::cumulative::CrossPartitionCombiner;
    use smelt_runtime::maintenance_driver::repair_augmented_model_sql;

    let sql = "SELECT customer_id, MAX_BY(amount, order_date) AS max_val FROM \
               smelt.sources.raw.orders GROUP BY customer_id";

    let unchanged = repair_augmented_model_sql(sql, &[]).expect("empty state must round-trip");
    assert_eq!(unchanged, sql);

    let state_columns = vec![
        StateColumn {
            name: "max_val__v".to_string(),
            per_partition_expr: "MAX_BY(amount, order_date)".to_string(),
            combiner: CrossPartitionCombiner::PlainOverwrite,
        },
        StateColumn {
            name: "max_val__o".to_string(),
            per_partition_expr: "MAX(order_date)".to_string(),
            combiner: CrossPartitionCombiner::Max,
        },
    ];
    let augmented =
        repair_augmented_model_sql(sql, &state_columns).expect("well-formed SQL must augment");
    assert!(
        augmented.contains(", MAX_BY(amount, order_date) AS max_val__v"),
        "{augmented}"
    );
    assert!(
        augmented.contains(", MAX(order_date) AS max_val__o"),
        "{augmented}"
    );

    let err = repair_augmented_model_sql("not even sql (((", &state_columns)
        .expect_err("unparseable SQL must refuse, not mangle");
    assert!(err.to_string().contains("could not be parsed"), "{err}");
}

/// `repair_candidate_select_carries_hidden_state_columns` (P10 test 3): a
/// candidate select built over the augmented SQL projects the state column
/// aliases, so a repair `INSERT` matches the fold-created table's column
/// list.
#[test]
fn repair_candidate_select_carries_hidden_state_columns() {
    use smelt_logical::analysis::decomposed_state::StateColumn;
    use smelt_logical::rules::cumulative::CrossPartitionCombiner;
    use smelt_runtime::maintenance_driver::repair_augmented_model_sql;

    let sql = "SELECT customer_id, MAX_BY(amount, order_date) AS max_val FROM \
               smelt.sources.raw.orders GROUP BY customer_id";
    let state_columns = vec![
        StateColumn {
            name: "max_val__v".to_string(),
            per_partition_expr: "MAX_BY(amount, order_date)".to_string(),
            combiner: CrossPartitionCombiner::PlainOverwrite,
        },
        StateColumn {
            name: "max_val__o".to_string(),
            per_partition_expr: "MAX(order_date)".to_string(),
            combiner: CrossPartitionCombiner::Max,
        },
    ];
    let augmented = repair_augmented_model_sql(sql, &state_columns).expect("must augment");

    let key = vec!["customer_id".to_string()];
    let affected_keys_select = repair_affected_keys_select(
        "main.sources_raw_orders",
        &key,
        None,
        &Region {
            start: "'2025-01-12'".to_string(),
            end: "'2025-01-13'".to_string(),
        },
    );
    let candidate = repair_candidate_select(&augmented, &key, &affected_keys_select);

    // `repair_candidate_select` wraps its input verbatim in `SELECT
    // __smelt_repair_candidate.*` — the augmented input's extra select
    // items flow through the wildcard, so the state aliases only need to
    // appear in the wrapped subquery text.
    assert!(candidate.contains("AS max_val__v"), "{candidate}");
    assert!(candidate.contains("AS max_val__o"), "{candidate}");
}

/// `diff_patch_compared_columns_include_hidden_state` (P10 test 4): the
/// compared-column set handed to `emit_diff_patch` for a state-bearing cell
/// contains the state column names, so the emitted suppression predicate
/// mentions them.
#[test]
fn diff_patch_compared_columns_include_hidden_state() {
    let table = "main.customer_max_amount";
    let key = vec!["customer_id".to_string()];
    let mut compared_columns = vec!["max_val".to_string()];
    compared_columns.extend(["max_val__v".to_string(), "max_val__o".to_string()]);

    let statements = smelt_logical::maintenance::emit::emit_diff_patch(
        table,
        "__smelt_diff_patch_customer_max_amount",
        &key,
        "SELECT customer_id, 1 AS max_val, 1 AS max_val__v, DATE '2025-01-01' AS max_val__o",
        &compared_columns,
        "TRUE",
        &DeleteLeg::Omitted {
            why: "test fixture — delete leg irrelevant to this assertion".to_string(),
        },
        smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
    );
    let full_text: String = statements
        .statements
        .iter()
        .map(|s| s.sql.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(full_text.contains("max_val__v"), "{full_text}");
    assert!(full_text.contains("max_val__o"), "{full_text}");
}

// ── DuckDB-backed legs (5, 6) ────────────────────────────────────────────
//
// A real `execute_project` run over a staged project: creation folds (there
// is nothing to repair yet), then a retraction in one group is repaired by
// the per-group recompute — never by the fold, whose combiner (`MAX`) cannot
// undo a retracted contribution.

/// The mutable, clocked source the repair fixture folds over. Clocked
/// (`timeseries:`) so `derive_model_maintenance_plan` derives no
/// `UpstreamMutation` trigger for it — its post-creation mutations are the
/// repair cell's job on the model's own `NewData` trigger, not an
/// enrichment cell's.
pub const ORDERS_SOURCE_YML: &str = r#"description: Mutable order snapshot
columns:
- name: order_id
  type: INTEGER
- name: customer_id
  type: INTEGER
- name: amount
  type: DECIMAL(10,2)
- name: order_date
  type: TIMESTAMP
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
unique_key: [order_id]
mutation_profile:
  kind: mutable_snapshot
"#;

/// The keyed repair fixture: `MAX` per `customer_id` over the mutable
/// source, with an explicit 3-day Form B band on `order_date` — the band
/// discharges the repair family's bounded-per-group-read obligation and
/// fixes the derived `ScanClamp` at `before = 3 days`.
pub const REPAIR_FIXTURE_SQL: &str = "SELECT customer_id, MAX(amount) AS max_amount \
     FROM smelt.sources.raw.orders \
     WHERE order_date BETWEEN TIMESTAMP '2025-01-14' - INTERVAL '3 days' AND TIMESTAMP \
     '2025-01-14' \
     GROUP BY customer_id";

pub const REPAIR_FIXTURE_FILE: &str = "---\n\
     materialization: table\n\
     refresh: incremental\n\
     grain: key\n\
     unique_key: customer_id\n\
     ---\n";

/// Same fixture, pinned to `write: diff_patch` — routes through
/// [`smelt_runtime::maintenance_driver::execute_diff_patch`] instead of the
/// repair family's own targeted delete+insert.
pub const REPAIR_FIXTURE_DIFF_PATCH_FILE: &str = "---\n\
     materialization: table\n\
     refresh: incremental\n\
     grain: key\n\
     unique_key: customer_id\n\
     maintenance:\n\
     \x20\x20cells:\n\
     \x20\x20- on: raw.orders\n\
     \x20\x20\x20\x20columns: [max_amount]\n\
     \x20\x20\x20\x20write: diff_patch\n\
     ---\n";

/// The full-refresh oracle for [`REPAIR_FIXTURE_SQL`], against the physical
/// source table.
pub const REPAIR_FIXTURE_ORACLE: &str = "SELECT customer_id, MAX(amount) AS max_amount \
     FROM main.sources_raw_orders \
     WHERE order_date BETWEEN TIMESTAMP '2025-01-14' - INTERVAL '3 days' AND TIMESTAMP \
     '2025-01-14' \
     GROUP BY customer_id";

pub fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) {
    std::fs::create_dir_all(dst).expect("create dst dir");
    for entry in std::fs::read_dir(src).expect("read src dir") {
        let entry = entry.expect("dir entry");
        let file_type = entry.file_type().expect("file type");
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path);
        } else {
            std::fs::copy(entry.path(), &dst_path).expect("copy file");
        }
    }
}

/// Stage `examples/timeseries` into `project_dir`, adding the mutable
/// `raw.orders` source and the keyed repair model.
pub fn stage_repair_project(project_dir: &std::path::Path) {
    stage_repair_project_with_file(project_dir, REPAIR_FIXTURE_FILE);
}

/// Same staging, with the caller's own model frontmatter (e.g.
/// [`REPAIR_FIXTURE_DIFF_PATCH_FILE`]) — the write-pin variant reuses every
/// other leg of the fixture (source, seed data, oracle) verbatim.
pub fn stage_repair_project_with_file(project_dir: &std::path::Path, model_file: &str) {
    let source_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");
    copy_dir_recursive(&source_dir, project_dir);
    std::fs::write(
        project_dir.join("models/sources/raw/orders.yml"),
        ORDERS_SOURCE_YML,
    )
    .expect("write orders source yml");
    std::fs::write(
        project_dir.join("models/customer_max_amount.sql"),
        format!("{model_file}{REPAIR_FIXTURE_SQL}\n"),
    )
    .expect("write repair model fixture");
}

/// Seed the physical source table. Customer 1's group has two rows inside
/// the widened affected-key window (`order_date >= run_start − 3 days`);
/// customer 2's single row sits at `2025-01-11`, inside the model's own band
/// but OUTSIDE run 2's affected-key window — so it is provably untouched by
/// the repair's targeted `DELETE`/`INSERT`.
pub async fn seed_orders(backend: &dyn smelt_backend::Backend) {
    backend
        .execute_sql(
            "CREATE TABLE main.sources_raw_orders (order_id INTEGER, customer_id INTEGER, \
             amount DECIMAL(10,2), order_date TIMESTAMP)",
        )
        .await
        .expect("create orders source table");
    backend
        .execute_sql(
            "INSERT INTO main.sources_raw_orders VALUES \
             (1, 1, 100.00, TIMESTAMP '2025-01-13 10:00:00'), \
             (2, 1, 50.00, TIMESTAMP '2025-01-13 11:00:00'), \
             (3, 2, 70.00, TIMESTAMP '2025-01-11 10:00:00')",
        )
        .await
        .expect("seed orders");
}

pub fn build_db_and_graph(
    project_dir: &std::path::Path,
    config: &smelt_core::config::Config,
) -> (
    std::sync::Arc<tokio::sync::Mutex<smelt_db::Database>>,
    std::sync::Arc<tokio::sync::Mutex<smelt_core::graph::DependencyGraph>>,
) {
    use smelt_core::ModelDiscovery;
    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
    let sql_models = discovery.discover_models().expect("discover_models");

    let mut db = smelt_db::Database::default();
    let project = db.set_project_input(project_dir.to_path_buf(), String::new());
    let source_files: Vec<_> = sql_models
        .iter()
        .map(|m| db.set_source_file(m.path.clone(), m.content.clone(), project_dir.to_path_buf()))
        .collect();
    db.set_workspace(source_files, vec![project]);
    db.set_active_target(
        config
            .target
            .clone()
            .map(|t| std::sync::Arc::from(t.as_str())),
    );

    let graph = smelt_core::graph::DependencyGraph::build(sql_models, None).expect("build graph");

    (
        std::sync::Arc::new(tokio::sync::Mutex::new(db)),
        std::sync::Arc::new(tokio::sync::Mutex::new(graph)),
    )
}

pub fn select_request(
    target: &str,
    model: &str,
    start: &str,
    end: &str,
) -> smelt_runtime::types::ExecuteRequest {
    smelt_runtime::types::ExecuteRequest {
        target: target.to_string(),
        select: vec![model.to_string()],
        exclude: vec![],
        start: Some(start.to_string()),
        end: Some(end.to_string()),
        batch_size_days: None,
        per_partition: false,
        full_refresh: false,
        dry_run: false,
        enforce_safety: false,
        allow_column_removal: false,
        allow_full_refresh: false,
        ephemeral_seed_ctes: vec![],
        run_checks: false,
        checks: vec![],
        jobs: None,
        retry_max: None,
        retry_backoff_ms: None,
        resume: false,
        technique_overrides: vec![],
    }
}

pub struct DuckDbBackendFactory {
    pub db_path: std::path::PathBuf,
}

impl smelt_runtime::execute::BackendFactory for DuckDbBackendFactory {
    fn create<'a>(
        &'a self,
        _target_name: &'a str,
        target_config: &'a smelt_core::config::Target,
        _project_dir: &'a std::path::Path,
    ) -> smelt_runtime::execute::BackendFuture<'a> {
        let path = self.db_path.clone();
        let schema = target_config.schema.clone();
        Box::pin(async move {
            let backend = smelt_backend_duckdb::DuckDbBackend::new(&path, &schema)
                .await
                .map_err(|e| anyhow::anyhow!("DuckDB init failed: {}", e))?;
            Ok(Box::new(backend) as Box<dyn smelt_backend::Backend>)
        })
    }
}

/// Scalar `SELECT` returning one DECIMAL rendered as text, for
/// bit-identity assertions that do not depend on Arrow decimal decoding.
pub async fn scalar_text(backend: &dyn smelt_backend::Backend, sql: &str) -> String {
    let batches = backend.execute_sql(sql).await.expect("query");
    let batch = batches.first().expect("one batch");
    assert_eq!(batch.num_rows(), 1, "expected exactly one row for: {sql}");
    let col = batch.column(0);
    arrow::util::display::array_value_to_string(col, 0).expect("render value")
}

async fn run_repair_fixture(
    project_dir: &std::path::Path,
    db_path: &std::path::Path,
) -> smelt_runtime::types::RunOutcome {
    run_repair_fixture_with_file(project_dir, db_path, REPAIR_FIXTURE_FILE).await
}

async fn run_repair_fixture_with_file(
    project_dir: &std::path::Path,
    db_path: &std::path::Path,
    model_file: &str,
) -> smelt_runtime::types::RunOutcome {
    use std::sync::Arc;
    stage_repair_project_with_file(project_dir, model_file);
    let config = Arc::new(smelt_core::config::Config::load(project_dir).expect("load smelt.yml"));

    {
        let backend = smelt_backend_duckdb::DuckDbBackend::new(db_path, "main")
            .await
            .expect("open duckdb");
        seed_orders(&backend).await;
    }

    // Run 1: creation. There is nothing to repair yet, so the fold's own
    // create path materializes the table.
    {
        let (db, graph) = build_db_and_graph(project_dir, &config);
        smelt_runtime::execute_project(
            "repair-run-1".to_string(),
            select_request("dev", "customer_max_amount", "2025-01-11", "2025-01-14"),
            Arc::clone(&config),
            graph,
            db,
            project_dir,
            &DuckDbBackendFactory {
                db_path: db_path.to_path_buf(),
            },
            &smelt_runtime::NoOpReporter,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("first run (create) must succeed");
    }

    // The retraction: customer 1's top-of-group contribution is corrected
    // downward in place. `MAX` cannot undo it — a fold would leave 100.00.
    {
        let backend = smelt_backend_duckdb::DuckDbBackend::new(db_path, "main")
            .await
            .expect("reopen duckdb");
        use smelt_backend::Backend;
        backend
            .execute_sql("UPDATE main.sources_raw_orders SET amount = 10.00 WHERE order_id = 1")
            .await
            .expect("retract");
    }

    // Run 2: the affected-key window is `order_date >= 2025-01-16 − 3 days`
    // (= 2025-01-13), so customer 1's group is affected and customer 2's
    // (2025-01-11) is not.
    let (db, graph) = build_db_and_graph(project_dir, &config);
    smelt_runtime::execute_project(
        "repair-run-2".to_string(),
        select_request("dev", "customer_max_amount", "2025-01-16", "2025-01-17"),
        Arc::clone(&config),
        graph,
        db,
        project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.to_path_buf(),
        },
        &smelt_runtime::NoOpReporter,
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("second run (per-group recompute) must succeed")
}

// ── 5 ────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn per_group_recompute_repairs_only_the_affected_group() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("run.duckdb");

    let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    drop(backend);

    let outcome = run_repair_fixture(&project_dir, &db_path).await;
    let record = outcome
        .models
        .get("customer_max_amount")
        .expect("customer_max_amount ran");
    assert_eq!(
        record.strategy, "per_group_recompute",
        "the retraction must dispatch the repair family, not the fold"
    );

    let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
        .await
        .expect("reopen duckdb");
    let repaired = scalar_text(
        &backend,
        "SELECT max_amount FROM main.customer_max_amount WHERE customer_id = 1",
    )
    .await;
    assert_eq!(
        repaired, "50.00",
        "the affected group must be recomputed whole — a fold would have left the retracted \
         100.00 in place"
    );
    let untouched = scalar_text(
        &backend,
        "SELECT max_amount FROM main.customer_max_amount WHERE customer_id = 2",
    )
    .await;
    assert_eq!(
        untouched, "70.00",
        "a group outside the affected-key window must be bit-identical after the repair"
    );
}

// ── 6 ────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn per_group_recompute_matches_full_refresh_after_retraction() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("run.duckdb");

    let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    drop(backend);

    run_repair_fixture(&project_dir, &db_path).await;

    let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
        .await
        .expect("reopen duckdb");
    assert!(
        multiset_equal(
            &backend,
            "SELECT customer_id, max_amount FROM main.customer_max_amount",
            REPAIR_FIXTURE_ORACLE,
        )
        .await,
        "the incremental state after the repair must equal a full refresh over the same inputs"
    );
}

// ── 7 ────────────────────────────────────────────────────────────────────
/// A `write: diff_patch` pin over the same repair fixture: run 2 dispatches
/// `execute_diff_patch` instead of the repair family's own targeted
/// delete+insert, and the resulting state still equals a full refresh over
/// the same inputs — the write leg differs, the equivalence invariant does
/// not.
#[tokio::test]
async fn diff_patch_execution_sends_the_emitted_group() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("run.duckdb");

    let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    drop(backend);

    let outcome =
        run_repair_fixture_with_file(&project_dir, &db_path, REPAIR_FIXTURE_DIFF_PATCH_FILE).await;
    let record = outcome
        .models
        .get("customer_max_amount")
        .expect("customer_max_amount ran");
    assert_eq!(
        record.strategy, "diff_patch",
        "the diff_patch pin must dispatch the diff-patch write, not the repair family's own \
         targeted delete+insert"
    );

    let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
        .await
        .expect("reopen duckdb");
    let repaired = scalar_text(
        &backend,
        "SELECT max_amount FROM main.customer_max_amount WHERE customer_id = 1",
    )
    .await;
    assert_eq!(
        repaired, "50.00",
        "the affected group must be recomputed whole via the diff-patch update leg"
    );
    assert!(
        multiset_equal(
            &backend,
            "SELECT customer_id, max_amount FROM main.customer_max_amount",
            REPAIR_FIXTURE_ORACLE,
        )
        .await,
        "the incremental state after a diff_patch repair must equal a full refresh over the \
         same inputs"
    );
}

// ── P9 test 5 ────────────────────────────────────────────────────────────
/// The gap phase 9 closes (`docs/specs/incremental_models.md` §"The repair
/// family" — "Obligation 7 over a `mutable_snapshot` source"): a key whose
/// ENTIRE window contribution is deleted from a `mutable_snapshot` source
/// leaves no row for a clamped current-source scan to select at all — the
/// group-grain fingerprint sidecar diff is what still discovers it, via its
/// own stored comparandum.
#[tokio::test]
async fn repair_fixes_a_key_whose_entire_window_contribution_was_deleted() {
    use smelt_backend::Backend;
    use std::sync::Arc;

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("run.duckdb");

    stage_repair_project(&project_dir);
    let config = Arc::new(smelt_core::config::Config::load(&project_dir).expect("load smelt.yml"));

    {
        let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open duckdb");
        backend
            .execute_sql(
                "CREATE TABLE main.sources_raw_orders (order_id INTEGER, customer_id INTEGER, \
                 amount DECIMAL(10,2), order_date TIMESTAMP)",
            )
            .await
            .expect("create orders source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_raw_orders VALUES \
                 (1, 1, 100.00, TIMESTAMP '2025-01-13 10:00:00'), \
                 (2, 1, 50.00, TIMESTAMP '2025-01-13 11:00:00'), \
                 (3, 2, 70.00, TIMESTAMP '2025-01-13 12:00:00')",
            )
            .await
            .expect("seed orders — customer 2's only row lies inside run 2's affected-key window");
    }

    // Run 1: creation.
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        smelt_runtime::execute_project(
            "repair-vanish-run-1".to_string(),
            select_request("dev", "customer_max_amount", "2025-01-11", "2025-01-14"),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &DuckDbBackendFactory {
                db_path: db_path.to_path_buf(),
            },
            &smelt_runtime::NoOpReporter,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("first run (create) must succeed");
    }

    {
        let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");
        let count = scalar_text(
            &backend,
            "SELECT COUNT(*) FROM main.customer_max_amount WHERE customer_id = 2",
        )
        .await;
        assert_eq!(count, "1", "customer 2 must exist after creation");
    }

    // Run 2 with NO source change: forces a live per-group-recompute pass
    // over the sidecar partition populated by run 1's own creation (task
    // 6) — its refresh re-stamps both customers' current digests, which is
    // the steady state the vanished-group leg below needs to actually
    // exercise the sidecar diff (P9 test 6 exercises the absent-comparandum
    // fallback directly, instead).
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        smelt_runtime::execute_project(
            "repair-vanish-run-2-populate".to_string(),
            select_request("dev", "customer_max_amount", "2025-01-16", "2025-01-17"),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &DuckDbBackendFactory {
                db_path: db_path.to_path_buf(),
            },
            &smelt_runtime::NoOpReporter,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("second run (steady-state sidecar refresh) must succeed");
    }

    // Delete customer 2's ENTIRE window contribution. No row survives for
    // ANY scan (clamped or not) to select — the sidecar's own stored
    // comparandum is the only thing left that can witness this.
    {
        let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");
        backend
            .execute_sql("DELETE FROM main.sources_raw_orders WHERE customer_id = 2")
            .await
            .expect("delete customer 2 entirely");
    }

    // Run 3: sidecar discovery is unbounded by the cell's clamp (P9) —
    // customer 2's vanished group must still be repaired away.
    let (db, graph) = build_db_and_graph(&project_dir, &config);
    let outcome = smelt_runtime::execute_project(
        "repair-vanish-run-3".to_string(),
        select_request("dev", "customer_max_amount", "2025-01-16", "2025-01-17"),
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.to_path_buf(),
        },
        &smelt_runtime::NoOpReporter,
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("third run (per-group recompute over the vanished group) must succeed");

    let record = outcome
        .models
        .get("customer_max_amount")
        .expect("customer_max_amount ran");
    assert_eq!(
        record.strategy, "per_group_recompute",
        "the run must dispatch the repair family"
    );

    let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
        .await
        .expect("reopen duckdb");
    let remaining = scalar_text(
        &backend,
        "SELECT COUNT(*) FROM main.customer_max_amount WHERE customer_id = 2",
    )
    .await;
    assert_eq!(
        remaining, "0",
        "customer 2's stale output row must be repaired away — its entire window contribution \
         was deleted from the source, invisible to any current-source scan"
    );

    let untouched = scalar_text(
        &backend,
        "SELECT max_amount FROM main.customer_max_amount WHERE customer_id = 1",
    )
    .await;
    assert_eq!(
        untouched, "100.00",
        "customer 1's group is untouched — no retraction happened for it"
    );
}

// ── P9 test 6 ────────────────────────────────────────────────────────────
/// Absent-comparandum degradation (P9 task 7,
/// `docs/specs/incremental_models.md` §"The repair family" — "Obligation 7
/// over a `mutable_snapshot` source"): with the sidecar partition dropped
/// outright, the affected set widens to every currently-observed group
/// PLUS every stored output group — a sound over-approximation that still
/// reaches a key that vanished while the comparandum was missing.
#[tokio::test]
async fn repair_covers_stored_output_keys_when_the_sidecar_is_absent() {
    use smelt_backend::Backend;
    use std::sync::Arc;

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("run.duckdb");

    stage_repair_project(&project_dir);
    let config = Arc::new(smelt_core::config::Config::load(&project_dir).expect("load smelt.yml"));

    {
        let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open duckdb");
        backend
            .execute_sql(
                "CREATE TABLE main.sources_raw_orders (order_id INTEGER, customer_id INTEGER, \
                 amount DECIMAL(10,2), order_date TIMESTAMP)",
            )
            .await
            .expect("create orders source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_raw_orders VALUES \
                 (1, 1, 100.00, TIMESTAMP '2025-01-13 10:00:00'), \
                 (2, 1, 50.00, TIMESTAMP '2025-01-13 11:00:00'), \
                 (3, 2, 70.00, TIMESTAMP '2025-01-13 12:00:00')",
            )
            .await
            .expect("seed orders");
    }

    // Run 1: creation. Task 6 populates the sidecar as a byproduct — drop
    // it below to force the absent-comparandum leg, not merely a stale one.
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        smelt_runtime::execute_project(
            "repair-absent-run-1".to_string(),
            select_request("dev", "customer_max_amount", "2025-01-11", "2025-01-14"),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &DuckDbBackendFactory {
                db_path: db_path.to_path_buf(),
            },
            &smelt_runtime::NoOpReporter,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("first run (create) must succeed");
    }

    // Delete customer 2 entirely, THEN drop the sidecar partition outright
    // — the comparandum is missing, not merely stale.
    {
        let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");
        backend
            .execute_sql("DELETE FROM main.sources_raw_orders WHERE customer_id = 2")
            .await
            .expect("delete customer 2 entirely");
        backend
            .execute_sql("DELETE FROM main._smelt_fingerprint_sidecar")
            .await
            .expect("drop the sidecar partition entirely");
    }

    let (db, graph) = build_db_and_graph(&project_dir, &config);
    let outcome = smelt_runtime::execute_project(
        "repair-absent-run-2".to_string(),
        select_request("dev", "customer_max_amount", "2025-01-16", "2025-01-17"),
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.to_path_buf(),
        },
        &smelt_runtime::NoOpReporter,
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("second run (absent-comparandum repair) must succeed");

    let record = outcome
        .models
        .get("customer_max_amount")
        .expect("customer_max_amount ran");
    assert_eq!(
        record.strategy, "per_group_recompute",
        "the run must dispatch the repair family"
    );

    let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
        .await
        .expect("reopen duckdb");
    let remaining = scalar_text(
        &backend,
        "SELECT COUNT(*) FROM main.customer_max_amount WHERE customer_id = 2",
    )
    .await;
    assert_eq!(
        remaining, "0",
        "even with the sidecar comparandum entirely missing, the run must still reach customer \
         2's vanished group via the stored-output-key over-approximation"
    );
}

/// `EXCEPT ALL` both ways — the Link-C multiset oracle this workspace's
/// other equivalence legs use (`statement_parity.rs::multiset_equal`).
pub async fn multiset_equal(
    backend: &dyn smelt_backend::Backend,
    left_sql: &str,
    right_sql: &str,
) -> bool {
    except_all_count(backend, left_sql, right_sql).await == 0
        && except_all_count(backend, right_sql, left_sql).await == 0
}

async fn except_all_count(
    backend: &dyn smelt_backend::Backend,
    left_sql: &str,
    right_sql: &str,
) -> i64 {
    use arrow::array::{Array, Int64Array};
    let sql = format!("SELECT COUNT(*) FROM (({left_sql}) EXCEPT ALL ({right_sql})) AS d");
    let batches = backend.execute_sql(&sql).await.expect("except all query");
    let batch = batches.first().expect("one batch");
    let col = batch
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("count column");
    col.value(0)
}

fn no_retry_policy() -> smelt_runtime::RetryPolicy<'static> {
    smelt_runtime::RetryPolicy {
        retry_max: 0,
        base_backoff_ms: 0,
        run_id: "keyless-recompute-test",
        model_name: "events_region",
        reporter: &smelt_runtime::NoOpReporter,
    }
}

/// Phase 27c (`docs/outcomes/20260815-definition-delta-migrate/
/// phases/27c-plan.md`): the keyless (whole-row) staged-candidate
/// conditional write against a real DuckDB backend — an unchanged candidate
/// must leave the target's stored rows untouched (zero deleted/inserted),
/// asserted via a frozen BEFORE snapshot rather than a rowid/`ctid`-style
/// observable DuckDB does not expose.
#[tokio::test]
async fn keyless_recompute_skips_the_write_when_the_candidate_matches_stored_state() {
    use smelt_backend::Backend;
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("run.duckdb");
    let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    backend
        .execute_sql("CREATE TABLE main.events_region (event_id INTEGER, tag VARCHAR)")
        .await
        .expect("create target table");
    backend
        .execute_sql("INSERT INTO main.events_region VALUES (1, 'a'), (2, 'b')")
        .await
        .expect("seed target table");
    backend
        .execute_sql("CREATE TABLE main.events_region_before AS SELECT * FROM main.events_region")
        .await
        .expect("snapshot before");

    // Byte-identical to what is already stored — the diff sentinel must be
    // empty, so neither the DELETE nor the INSERT touches a row.
    let candidate_select = "SELECT event_id, tag FROM main.events_region";
    let result = smelt_runtime::maintenance_driver::execute_staged_keyless_recompute(
        &backend,
        "main",
        "events_region",
        candidate_select,
        &no_retry_policy(),
    )
    .await
    .expect("keyless recompute must succeed");
    assert_eq!(
        result.row_count, 2,
        "the unchanged region's stored row count is unaffected"
    );

    assert!(
        multiset_equal(
            &backend,
            "SELECT * FROM main.events_region",
            "SELECT * FROM main.events_region_before"
        )
        .await,
        "an unchanged candidate must leave the stored state exactly as the BEFORE snapshot \
         recorded it"
    );
}

/// The equivalence leg: a genuinely changed candidate must rewrite the
/// region so stored state equals the candidate afterwards.
#[tokio::test]
async fn keyless_recompute_rewrites_the_region_when_the_candidate_differs() {
    use smelt_backend::Backend;
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("run.duckdb");
    let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    backend
        .execute_sql("CREATE TABLE main.events_region (event_id INTEGER, tag VARCHAR)")
        .await
        .expect("create target table");
    backend
        .execute_sql("INSERT INTO main.events_region VALUES (1, 'a'), (2, 'b')")
        .await
        .expect("seed target table");

    // The candidate drops event 2 and changes event 1's tag — a genuinely
    // different whole-row multiset.
    let candidate_select = "SELECT * FROM (VALUES (1, 'z')) AS t(event_id, tag)";
    smelt_runtime::maintenance_driver::execute_staged_keyless_recompute(
        &backend,
        "main",
        "events_region",
        candidate_select,
        &no_retry_policy(),
    )
    .await
    .expect("keyless recompute must succeed");

    assert!(
        multiset_equal(
            &backend,
            "SELECT * FROM main.events_region",
            candidate_select
        )
        .await,
        "a changed candidate must leave stored state equal to the candidate"
    );
}
