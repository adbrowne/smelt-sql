/// The open write-pattern registry's `write:` pin, end-to-end
/// (`docs/specs/incremental_models.md` §"Per-cell write addressing" →
/// "User pins"; `docs/plans/20260715-composed-axes-conditional-
/// maintenance.md` Phase R1). Unlike an earlier version of this module,
/// these tests do not call [`resolve_write_pin`] and
/// `emit_delete_insert`/`resolve_cell_technique_with_write_pin` as two
/// disconnected function calls that happen to agree by construction — the
/// resolved [`smelt_logical::maintenance::WritePattern`] returned by
/// [`resolve_write_pin`] is fed directly into
/// [`smelt_logical::maintenance::choice::resolve_cell_choice`] (the real
/// technique-choice resolver `choice.rs` documents as the module a
/// `write:` pin must constrain) and into
/// [`resolve_cell_technique_with_write_pin`] (the real runtime driver
/// resolver `maintenance_driver.rs` documents the same way) — the same two
/// functions the review flagged as validating-and-discarding the pin. Each
/// fixture is chosen so the pin changes what these resolvers pick relative
/// to their own unpinned default: a non-vacuous proof the pin is actually
/// consulted, not merely accepted.
use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_logical::maintenance::choice::{
    effective_override, resolve_cell_choice, ChosenTechnique,
};
use smelt_logical::maintenance::emit::{emit_delete_insert, MaintenanceDialect, Region};
use smelt_logical::maintenance::{
    lookup_write_pattern, resolve_write_pin, BackendWriteCapabilities, Corner, MaintenancePlan,
    OutputContractFacts, PartitionLocal, PlanCell, RowIdentity, RowIdentityVerdict, Technique,
    Trigger,
};

use super::{admitted_plan, resolve_cell_technique_with_write_pin, ResolvedTechnique};

/// A composed model's mutation-trigger cell whose derived plan admits
/// `Technique::KeyedFold` (the fold-a-delta corner — `RowIdentity::
/// Key`, the shape a `grain: key` + `timeseries:` composed model
/// derives for an upstream source mutation) — the default,
/// UNPINNED choice this fixture exists to be overridden away from.
fn composed_keyed_fold_plan(source: &str) -> MaintenancePlan {
    MaintenancePlan {
        cells: vec![PlanCell {
            group: "{*}".to_string(),
            trigger: Trigger::UpstreamMutation {
                source: source.to_string(),
            },
            corner: Corner::FoldDelta,
            technique: Technique::KeyedFold,
            partition_local: PartitionLocal::Yes,
            scans: vec![],
            ledger_catch_up: false,
            row_identity: RowIdentityVerdict {
                identity: RowIdentity::Key(vec!["id".to_string()]),
                proven_mismatch: None,
            },
            skeleton_source_closure: None,
            fingerprint_projections: std::collections::BTreeMap::new(),
            key_scope: None,
            state_downgrade: None,
        }],
        refusals: vec![],
        key_locality: None,
    }
}

/// Pinning `write: region` on a composed model's mutation cell —
/// admitted by the plan as `KeyedFold`, not `region`'s
/// `DeleteInsert`/region-recompute corner — resolves against the open
/// registry (the pattern only requires a declared partition axis, which
/// a composed key+timeseries output declares) and then, fed into the
/// real `resolve_cell_choice`, overrides the cell's own admitted
/// technique: the pin changes the resolved choice from `Admitted
/// (KeyedFold)` (what an unpinned resolution picks) to
/// `RegionRecompute` — proving the pin is actually consulted, not
/// merely validated and discarded. The resolved `RegionRecompute`
/// choice is then lowered through the SAME `emit_delete_insert` emitter
/// `Technique::DeleteInsert` cells use and actually executed against a
/// real DuckDB backend, matching a hand-written full-refresh oracle.
#[tokio::test]
async fn pinning_region_on_composed_mutation_cell_overrides_keyed_fold_to_delete_insert() {
    let source = "sources.raw_events";
    let plan = composed_keyed_fold_plan(source);
    let trigger = Trigger::UpstreamMutation {
        source: source.to_string(),
    };

    // 0. Prove the fixture is non-vacuous: absent a write pin, the real
    //    resolver picks the cell's own admitted `KeyedFold`, not region
    //    recompute.
    let unpinned = resolve_cell_choice(
        plan.cell_for(&trigger),
        &trigger,
        &effective_override(None, &[], "unused", &[]),
        None,
        true,
    )
    .expect("unpinned resolution must not refuse");
    assert_eq!(
        unpinned,
        ChosenTechnique::Admitted(Technique::KeyedFold),
        "the unpinned default must be the cell's own admitted technique, not region \
         recompute — otherwise the pin below can't be proven to have changed anything"
    );

    // 1. Resolve the pin against the registry: a composed (key +
    //    partition-axis) output admits `region` (it only requires a
    //    declared partition axis).
    let facts = OutputContractFacts {
        has_identity: true,
        has_partition_axis: true,
    };
    let backend_caps = BackendWriteCapabilities {
        supports_merge: true,
        supports_column_scoped_merge: true,
    };
    let resolved_pattern = resolve_write_pin(
        "UpstreamMutation",
        "region",
        "duckdb",
        facts,
        backend_caps,
        |_pattern| Ok(()),
    )
    .expect("a `region` pin on a partition-axis output must resolve");
    assert_eq!(resolved_pattern.name, "region");
    // `lookup_write_pattern` is the same registry lookup
    // `resolve_cell_technique`'s/`resolve_cell_choice`'s production
    // call sites use to turn a stored `cells[].write: String` back into
    // a `&'static WritePattern` — exercised here instead of just
    // reusing `resolved_pattern` directly, so this test also proves the
    // production lookup path resolves to the identical entry.
    let looked_up = lookup_write_pattern("region").expect("registered pattern");
    assert_eq!(looked_up.name, resolved_pattern.name);

    // 2. Feed the resolved, validated pattern into the REAL
    //    technique-choice resolver — this is the wiring the review
    //    found missing: the pin must actually change what this
    //    function picks, not just have been checked upstream.
    let pinned = resolve_cell_choice(
        plan.cell_for(&trigger),
        &trigger,
        &effective_override(None, &[], "unused", &[]),
        Some(looked_up),
        true,
    )
    .expect("a region pin on a partition-axis output must resolve, not refuse");
    assert_eq!(
        pinned,
        ChosenTechnique::RegionRecompute,
        "the write pin must override the cell's own admitted KeyedFold technique"
    );
    assert_ne!(
        pinned, unpinned,
        "the pin must actually change the outcome relative to the unpinned default — \
         otherwise it is validated and ignored, not consulted"
    );

    // 3. `ChosenTechnique::RegionRecompute` lowers to the SAME
    //    `emit_delete_insert` emitter `Technique::DeleteInsert` cells
    //    use (`incremental_models.md` §"Statement emission (single
    //    owner)") — actually executed against a real DuckDB backend and
    //    checked against a hand-written full-refresh oracle.
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    backend
        .execute_sql("CREATE TABLE main.daily_totals (d DATE, total DOUBLE)")
        .await
        .expect("create target table");
    backend
        .execute_sql(
            "INSERT INTO main.daily_totals VALUES \
             (DATE '2024-01-01', 999.0), \
             (DATE '2024-01-02', 20.0)",
        )
        .await
        .expect("seed target table with a stale 2024-01-01 row");
    backend
        .execute_sql("CREATE TABLE main.raw_events (d DATE, amount DOUBLE)")
        .await
        .expect("create source table");
    backend
        .execute_sql(
            "INSERT INTO main.raw_events VALUES \
             (DATE '2024-01-01', 5.0), \
             (DATE '2024-01-01', 7.0)",
        )
        .await
        .expect("seed source table");

    let region = Region {
        start: "DATE '2024-01-01'".to_string(),
        end: "DATE '2024-01-02'".to_string(),
    };
    let body = "SELECT d, SUM(amount) AS total FROM main.raw_events WHERE d >= DATE '2024-01-01' \
         AND d < DATE '2024-01-02' GROUP BY d";
    let group = match pinned {
        ChosenTechnique::RegionRecompute => emit_delete_insert(
            "main.daily_totals",
            "d",
            &region,
            body,
            MaintenanceDialect::DuckDb,
        ),
        ChosenTechnique::Admitted(_) => {
            panic!("the pin must have resolved to RegionRecompute — asserted above")
        }
        ChosenTechnique::DiffPatch { .. } => {
            panic!("the pin must have resolved to RegionRecompute — asserted above")
        }
    };
    assert_eq!(
        group.statements.len(),
        2,
        "the region pattern's physical mechanism is exactly one DELETE + one INSERT"
    );
    assert!(group.statements[0].sql.starts_with("DELETE FROM"));
    assert!(group.statements[1].sql.starts_with("INSERT INTO"));
    assert!(group.transactional, "DELETE+INSERT must be one transaction");

    backend
        .execute_statement_group(&group)
        .await
        .expect("DELETE+INSERT region rewrite must succeed");

    let conn = duckdb::Connection::open(&db_path).expect("reconnect");
    let recomputed_total: f64 = conn
        .query_row(
            "SELECT total FROM main.daily_totals WHERE d = DATE '2024-01-01'",
            [],
            |row| row.get(0),
        )
        .expect("read recomputed total");
    assert_eq!(
        recomputed_total, 12.0,
        "the pinned region rewrite must replace the stale row with the recomputed total \
         (5.0 + 7.0), matching a full-refresh oracle over the same region"
    );

    let untouched_total: f64 = conn
        .query_row(
            "SELECT total FROM main.daily_totals WHERE d = DATE '2024-01-02'",
            [],
            |row| row.get(0),
        )
        .expect("read untouched row");
    assert_eq!(
        untouched_total, 20.0,
        "a region-scoped DELETE+INSERT must not touch rows outside the pinned region"
    );
}

/// A `write: keyed` pin on an identity-free output refuses at
/// resolution time — never silently falls back to `region` or any
/// other addressing (no substituted technique).
#[test]
fn pinning_keyed_on_identity_free_output_refuses_never_substitutes() {
    let facts = OutputContractFacts {
        has_identity: false,
        has_partition_axis: true,
    };
    let backend_caps = BackendWriteCapabilities {
        supports_merge: true,
        supports_column_scoped_merge: true,
    };
    let err = resolve_write_pin(
        "Backfill",
        "keyed",
        "duckdb",
        facts,
        backend_caps,
        |_pattern| Ok(()),
    )
    .expect_err("keyed must refuse on an identity-free output");
    assert!(err
        .to_string()
        .contains("MaintenanceWriteAddressingRefused"));
}

/// A pin that resolves cleanly against the registry (structural facts +
/// backend capability both satisfied) can still be refused one level
/// deeper by `resolve_cell_choice`, when the validated pattern's
/// selection isn't what THIS cell's derived plan actually admitted —
/// e.g. `write: column` validated fine against an identity-bearing
/// output, but the cell in hand only ever admitted `KeyedFold`, not
/// `ColumnScopedMerge`. Never a silent substitution to whatever WAS
/// admitted.
#[test]
fn pinning_column_on_a_keyed_fold_cell_refuses_at_the_choice_layer() {
    let source = "sources.raw_events";
    let plan = composed_keyed_fold_plan(source);
    let trigger = Trigger::UpstreamMutation {
        source: source.to_string(),
    };

    let facts = OutputContractFacts {
        has_identity: true,
        has_partition_axis: true,
    };
    let backend_caps = BackendWriteCapabilities {
        supports_merge: true,
        supports_column_scoped_merge: true,
    };
    let resolved_pattern = resolve_write_pin(
        "UpstreamMutation",
        "column",
        "duckdb",
        facts,
        backend_caps,
        |_pattern| Ok(()),
    )
    .expect("`column` resolves fine against the registry for an identity-bearing output");

    let err = resolve_cell_choice(
        plan.cell_for(&trigger),
        &trigger,
        &effective_override(None, &[], "unused", &[]),
        Some(resolved_pattern),
        true,
    )
    .expect_err("a registry-valid pin whose selection the cell never admitted must still refuse");
    assert!(
        err.to_string().contains("MaintenanceUnboundedFootprint"),
        "refusal must name the diagnostic family: {err}"
    );
}

/// The narrower runtime driver resolver
/// (`maintenance_driver::resolve_cell_technique_with_write_pin`) — the
/// second function the review named — is consulted the same way: a
/// `write: region` pin overrides a live `ColumnScopedMerge` cell's own
/// default to region recompute, a `write: column` pin reaffirms it, and
/// a pin selecting a technique this narrow (`ColumnScopedMerge` vs
/// region-only) resolver has no lowering for (`keyed`) refuses rather
/// than silently falling back.
#[test]
fn driver_resolve_cell_technique_consults_the_write_pin() {
    let plan = admitted_plan("users");
    let trigger = Trigger::UpstreamMutation {
        source: "users".to_string(),
    };

    // Unpinned default: the live, admitted ColumnScopedMerge cell.
    let unpinned = resolve_cell_technique_with_write_pin(&plan, &trigger, None, None, true)
        .expect("unpinned resolution must not refuse");
    assert_eq!(unpinned, ResolvedTechnique::ColumnScopedMerge);

    // `write: region` overrides that default to region recompute — a
    // real, non-vacuous behaviour change caused by the pin.
    let region_pattern = lookup_write_pattern("region").expect("registered pattern");
    let region_pinned =
        resolve_cell_technique_with_write_pin(&plan, &trigger, None, Some(region_pattern), true)
            .expect("a region pin must resolve, not refuse");
    assert_eq!(region_pinned, ResolvedTechnique::RegionRecompute);
    assert_ne!(region_pinned, unpinned);

    // `write: column` reaffirms the admitted, live technique.
    let column_pattern = lookup_write_pattern("column").expect("registered pattern");
    let column_pinned =
        resolve_cell_technique_with_write_pin(&plan, &trigger, None, Some(column_pattern), true)
            .expect("a column pin on an admitted, live cell must resolve");
    assert_eq!(column_pinned, ResolvedTechnique::ColumnScopedMerge);

    // `write: keyed` selects a technique (`KeyedFold`) this narrow
    // resolver has no lowering for — refuses fail-loud rather than
    // silently substituting a different technique than the one pinned.
    let keyed_pattern = lookup_write_pattern("keyed").expect("registered pattern");
    let err =
        resolve_cell_technique_with_write_pin(&plan, &trigger, None, Some(keyed_pattern), true)
            .expect_err("a pin selecting a technique this resolver can't lower must refuse");
    assert!(err.to_string().contains("MaintenanceUnboundedFootprint"));
}

/// Pins the resolvable set member `write: keyed` (selects
/// `Technique::KeyedFold`, this cell's own admitted technique) directly
/// through `lookup_write_pattern` + `resolve_cell_choice`: proves
/// `admits_write_selection`'s equality check on the exact-technique arm
/// (`selection == Technique(t)`, `t != ColumnScopedMerge`) admits when
/// the pinned technique equals the plan's admitted technique. Kills the
/// `admits_write_selection` `==` → `!=` mutant on that arm together with
/// `pinning_update_on_a_keyed_fold_cell_refuses` below.
#[test]
fn pinning_keyed_on_a_keyed_fold_cell_admits() {
    let source = "sources.raw_events";
    let plan = composed_keyed_fold_plan(source);
    let trigger = Trigger::UpstreamMutation {
        source: source.to_string(),
    };

    let keyed_pattern = lookup_write_pattern("keyed").expect("registered pattern");
    let chosen = resolve_cell_choice(
        plan.cell_for(&trigger),
        &trigger,
        &effective_override(None, &[], "unused", &[]),
        Some(keyed_pattern),
        true,
    )
    .expect("a keyed pin matching the cell's own admitted KeyedFold technique must resolve");
    assert_eq!(chosen, ChosenTechnique::Admitted(Technique::KeyedFold));
}

/// Pins `write: update` (selects `Technique::InPlaceUpdate`) against the
/// same `KeyedFold`-admitted plan: the pinned technique does NOT equal
/// the plan's admitted technique, so `admits_write_selection` must
/// refuse. With the `==` → `!=` mutant this would wrongly admit,
/// substituting a technique the plan never derived.
#[test]
fn pinning_update_on_a_keyed_fold_cell_refuses() {
    let source = "sources.raw_events";
    let plan = composed_keyed_fold_plan(source);
    let trigger = Trigger::UpstreamMutation {
        source: source.to_string(),
    };

    let update_pattern = lookup_write_pattern("update").expect("registered pattern");
    let err = resolve_cell_choice(
        plan.cell_for(&trigger),
        &trigger,
        &effective_override(None, &[], "unused", &[]),
        Some(update_pattern),
        true,
    )
    .expect_err(
        "an update pin selecting a technique the plan never admitted must refuse, not \
         substitute the admitted technique",
    );
    assert!(err.to_string().contains("MaintenanceUnboundedFootprint"));
}
