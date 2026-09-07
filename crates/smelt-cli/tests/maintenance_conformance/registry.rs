//! The maintenance-conformance divergence registry
//! (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 11):
//! named, tracked deviations from the equivalence invariant this suite
//! otherwise enforces unconditionally, governed the same way
//! `crates/smelt-db/tests/prop_helpers/known_unknowns.rs` governs its own
//! registry — `known_unknowns_staleness_report` is the pattern this module
//! mirrors (`crates/smelt-db/tests/type_property_tests.rs`).
//!
//! Two entry families:
//!
//! - **Adversarial-leaf entries** — one per
//!   [`smelt_maintenance_testkit::recipe::AdversarialLeaf`] variant. Over a
//!   deterministic sample drawn from `arb_adversarial_recipe()`, each leaf
//!   kind must be observed refusing (or collapsing to full-input recompute)
//!   at least once — the fail-closed behaviour
//!   `verdict.rs::adversarial_leaves_refuse_or_collapse_conservatively`
//!   already asserts holds for every case, restated here as a per-id
//!   staleness ledger.
//! - **`KnownBug` structural entries** — a production gap this suite
//!   discovered and deliberately did not fix yet, verified by a structural
//!   check against the exact call site named in the entry — the moment the
//!   gap closes, the check stops matching and this test reports it as
//!   stale, the signal to delete the entry and file the fix as its own
//!   change. No entry currently holds this status: the family's two prior
//!   members were both closed this way —
//!   `known_bug_technique_pin_inert` once `resolve_cell_choice`/
//!   `effective_override` wired the frontmatter pin ladder into the live
//!   call site (`crates/smelt-runtime/src/maintenance_driver.rs`), and
//!   `known_bug_incremental_path_skips_schema_snapshot` once the
//!   deployed-schema snapshot the definition-change trigger reads became
//!   current on the incremental path too
//!   (`docs/plans/20260809-sensitivity-precision.md` Phase 6,
//!   `crates/smelt-runtime/src/execute.rs`'s pre-schema-evolution-gate
//!   snapshot capture).
//!
//! A registry entry that never fired over the deterministic sample is
//! reported (`eprintln!`), never a test failure — warn-level by design, so
//! closing a gap doesn't turn the suite red until someone prunes the
//! registry (mirrors `known_unknowns_staleness_report`'s own doc comment).

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;
use std::collections::BTreeSet;

use smelt_maintenance_testkit::recipe::{arb_adversarial_recipe, AdversarialLeaf};
use smelt_maintenance_testkit::verdict::{classify_adversarial, stage_adversarial, Verdict};

/// Why a registered divergence exists — mirrors
/// `prop_helpers::divergences::DivergenceStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DivergenceStatus {
    /// A generated case exercising a documented fail-closed corner
    /// (`model_properties.md`/`incremental_models.md` §Known Divergences).
    Documented,
    /// A confirmed production gap, deliberately not fixed by this plan
    /// (this plan's "Deferred during implementation" section). Four prior
    /// members of this status were closed this way (see this module's own
    /// doc comment) — most recently
    /// `known_bug_repair_candidate_select_ignores_decomposed_state`, closed
    /// by widening the repair candidate/insert with the fold's own hidden
    /// state columns (`docs/outcomes/20260809-repair-family/phases/
    /// 10-plan.md`). Currently holds one member —
    /// `known_bug_keyed_upstream_partition_downstream_no_live_dispatch`
    /// (`docs/outcomes/20260809-output-delta-typing/phases/08-plan.md`) —
    /// kept alive as the governance machinery (`known_bug_still_reproduces`,
    /// the staleness-report match arm) for that and whichever production
    /// gap this suite discovers next.
    KnownBug,
}

struct DivergenceEntry {
    id: &'static str,
    description: &'static str,
    status: DivergenceStatus,
}

fn adversarial_leaf_id(leaf: AdversarialLeaf) -> &'static str {
    match leaf {
        AdversarialLeaf::OpaqueEventTime => "adversarial_opaque_event_time",
        AdversarialLeaf::IntersectBody => "adversarial_intersect_body",
        AdversarialLeaf::NondeterministicSkeleton => "adversarial_nondeterministic_skeleton",
        AdversarialLeaf::SymbolicIntervalBound => "adversarial_symbolic_interval_bound",
    }
}

fn registry() -> Vec<DivergenceEntry> {
    vec![
        DivergenceEntry {
            id: adversarial_leaf_id(AdversarialLeaf::OpaqueEventTime),
            description: "an opaque/unrecognised function call wrapping the event-time column \
                is classified Undecidable and must refuse or fully collapse, never optimistically \
                pass (model_properties.md event-time trace).",
            status: DivergenceStatus::Documented,
        },
        DivergenceEntry {
            id: adversarial_leaf_id(AdversarialLeaf::IntersectBody),
            description: "INTERSECT/EXCEPT collapse every payload column into one group \
                sensitive to every declared source (incremental_models.md §Known Divergences).",
            status: DivergenceStatus::Documented,
        },
        DivergenceEntry {
            id: adversarial_leaf_id(AdversarialLeaf::NondeterministicSkeleton),
            description: "a row-nondeterministic function (RANDOM()) in a skeleton/identity \
                position must refuse or fully collapse, never derive a stable targeted technique.",
            status: DivergenceStatus::Documented,
        },
        DivergenceEntry {
            id: adversarial_leaf_id(AdversarialLeaf::SymbolicIntervalBound),
            description: "a calendar-variable (INTERVAL '1 month') offset on the event-time \
                column cannot populate a Bounded scan window — NotDerivable, not an approximate \
                fixed-day guess (model_properties.md interval-literal parsing note).",
            status: DivergenceStatus::Documented,
        },
        DivergenceEntry {
            id: "known_bug_keyed_upstream_partition_downstream_no_live_dispatch",
            description: "a KeyedUpsert upstream feeding a grain: partition downstream has no \
                live key-addressed dispatch — the run loop's resolution/execution of that cell \
                is wired only inside the grain: key branch of smelt-runtime/src/execute.rs, so a \
                grain: partition downstream never reaches it (incremental_models.md §Known \
                Divergences).",
            status: DivergenceStatus::KnownBug,
        },
    ]
}

/// Sample `arb_adversarial_recipe()` deterministically, recording which
/// `Documented` entries' leaf kind was actually observed refusing or
/// collapsing to full recompute at least once.
fn fired_adversarial_ids() -> BTreeSet<&'static str> {
    let mut fired = BTreeSet::new();
    let mut runner = TestRunner::deterministic();
    let strat = arb_adversarial_recipe();

    const N: usize = 40;
    for i in 0..N {
        let recipe = strat.new_tree(&mut runner).unwrap().current();
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().join("project");
        let db_path = tmp.path().join("db.duckdb");

        let project = stage_adversarial(&recipe, &project_dir, &db_path)
            .unwrap_or_else(|e| panic!("case {i}: adversarial recipe {recipe:?} stage: {e}"));
        let verdict = classify_adversarial(&project, &recipe)
            .unwrap_or_else(|e| panic!("case {i}: adversarial recipe {recipe:?} classify: {e}"));

        // Every case must be fail-closed (refused, or admitted with only
        // full-input-recompute cells) — this is
        // `verdict.rs::adversarial_leaves_refuse_or_collapse_conservatively`'s
        // own assertion, restated so a regression here fails loudly instead
        // of silently under-counting which ids fired.
        let fail_closed = match &verdict {
            Verdict::Refused(diags) => !diags.is_empty(),
            Verdict::Admitted(plan) => plan
                .cells
                .iter()
                .all(|c| c.corner == smelt_logical::maintenance::Corner::RecomputeRegion),
        };
        assert!(
            fail_closed,
            "case {i}: adversarial recipe {recipe:?} was not fail-closed: {verdict:?}"
        );

        fired.insert(adversarial_leaf_id(recipe.leaf));
    }
    fired
}

/// Structurally verify each `KnownBug` entry still reproduces — grep
/// the exact call site each entry names for the literal text that makes the
/// gap true today. When a gap closes, the matching text disappears and
/// this returns `false`, which `divergence_registry_staleness_report`
/// reports (never fails) as a stale entry to prune.
fn known_bug_still_reproduces(id: &str) -> bool {
    match id {
        "known_bug_incremental_path_skips_schema_snapshot" => {
            // `save_deployed_schema` is called from exactly one place in
            // the execute pipeline (the full-refresh branch) — never a second call
            // site in the incremental branch.
            let src = concat!(
                include_str!("../../../smelt-runtime/src/execute/backend.rs"),
                include_str!("../../../smelt-runtime/src/execute/bootstrap.rs"),
                include_str!("../../../smelt-runtime/src/execute/key_addressed.rs"),
                include_str!("../../../smelt-runtime/src/execute/mod.rs"),
                include_str!("../../../smelt-runtime/src/execute/outcome.rs"),
                include_str!("../../../smelt-runtime/src/execute/plan.rs"),
                include_str!("../../../smelt-runtime/src/execute/project/mod.rs"),
                include_str!("../../../smelt-runtime/src/execute/project/dry_run.rs"),
                include_str!("../../../smelt-runtime/src/execute/retry.rs"),
                include_str!("../../../smelt-runtime/src/execute/sink.rs"),
                include_str!("../../../smelt-runtime/src/execute/sources.rs"),
                include_str!("../../../smelt-runtime/src/execute/targets.rs"),
                include_str!("../../../smelt-runtime/src/execute/window.rs"),
            );
            src.matches("save_deployed_schema").count() == 1
        }
        "known_bug_keyed_upstream_partition_downstream_no_live_dispatch" => {
            // `resolve_live_key_addressed_model_edge_cell` is consulted
            // from exactly one call site in the execute pipeline, inside the
            // `grain: key` (`plan_is_keyed`) branch — the moment a second
            // call site wires it into the `grain: partition` branch too,
            // this count changes and the entry goes stale.
            let src = concat!(
                include_str!("../../../smelt-runtime/src/execute/backend.rs"),
                include_str!("../../../smelt-runtime/src/execute/bootstrap.rs"),
                include_str!("../../../smelt-runtime/src/execute/key_addressed.rs"),
                include_str!("../../../smelt-runtime/src/execute/mod.rs"),
                include_str!("../../../smelt-runtime/src/execute/outcome.rs"),
                include_str!("../../../smelt-runtime/src/execute/plan.rs"),
                include_str!("../../../smelt-runtime/src/execute/project/mod.rs"),
                include_str!("../../../smelt-runtime/src/execute/project/dry_run.rs"),
                include_str!("../../../smelt-runtime/src/execute/retry.rs"),
                include_str!("../../../smelt-runtime/src/execute/sink.rs"),
                include_str!("../../../smelt-runtime/src/execute/sources.rs"),
                include_str!("../../../smelt-runtime/src/execute/targets.rs"),
                include_str!("../../../smelt-runtime/src/execute/window.rs"),
            );
            src.matches("resolve_live_key_addressed_model_edge_cell")
                .count()
                == 1
        }
        other => panic!("known_bug_still_reproduces: unhandled id {other:?}"),
    }
}

/// `divergence_registry_staleness_report` (plan Phase 11 TDD list):
/// registry entries that never fired in the deterministic sample are
/// reported, never a test failure.
#[test]
fn divergence_registry_staleness_report() {
    let entries = registry();
    let fired_adversarial = fired_adversarial_ids();

    for e in &entries {
        let fired = match e.status {
            DivergenceStatus::Documented => fired_adversarial.contains(e.id),
            DivergenceStatus::KnownBug => known_bug_still_reproduces(e.id),
        };
        if !fired {
            eprintln!(
                "warning: divergence-registry entry '{}' never fired ({}) — the gap it names \
                 may be closed; consider deleting the entry (or, for a KnownBug, filing its own \
                 fix): {}",
                e.id,
                match e.status {
                    DivergenceStatus::Documented => "no generated case exercised it",
                    DivergenceStatus::KnownBug => "structural check no longer matches",
                },
                e.description
            );
        }
    }

    // The registry itself must stay non-empty and every id unique — a
    // governance regression (an accidental duplicate/empty registry) fails
    // loudly rather than silently reporting nothing.
    let ids: BTreeSet<&str> = entries.iter().map(|e| e.id).collect();
    assert_eq!(
        ids.len(),
        entries.len(),
        "duplicate divergence-registry id — every entry must be uniquely named"
    );
    assert!(!entries.is_empty(), "divergence registry must not be empty");
}
