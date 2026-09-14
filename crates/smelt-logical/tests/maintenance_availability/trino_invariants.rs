//! Phase 5 (`docs/outcomes/20260913-trino-ledger/outcome.md`) — **absence ⇒
//! downgrade, never a refusal**, made exhaustive over [`Technique`] rather
//! than the three hand-picked cells
//! `resolution::trino_downgrades_every_dependent_cell_to_its_recompute_equivalent`
//! already covers.
//!
//! Every [`Technique`] variant, crossed with every [`KeyDiscovery`] shape a
//! `PerGroupRecompute` cell may carry (plus the no-`key_scope` shape every
//! other technique carries): under Trino's empty availability, a cell whose
//! `required_state_structure` is `None` survives untouched; every other cell
//! resolves to [`recompute_equivalent`], carries a [`StateDowngrade`] naming
//! the ideal technique and the missing structure, and — the reachability
//! half — its OWN `required_state_structure` is `None` after resolution, so
//! no execution path reaches a builder for it.

use smelt_core::config::WarehouseTables;
use smelt_dialect::SqlDialect;
use smelt_logical::maintenance::availability::{
    realisable_state_structures, recompute_equivalent, required_state_structure,
    resolve_availability, StateAvailability,
};
use smelt_logical::maintenance::{Corner, KeyDiscovery, KeyScope, PlanCell, Technique};

use super::base_cell;

/// Every [`Technique`] variant. The `match` in
/// [`every_technique_downgrades_or_needs_nothing_on_trino`] over this array
/// is exhaustive — a new variant is a compile error there, not a silently
/// uncovered case.
const ALL_TECHNIQUES: [Technique; 6] = [
    Technique::DeleteInsert,
    Technique::KeyedFold,
    Technique::ColumnScopedMerge,
    Technique::InPlaceUpdate,
    Technique::PerGroupRecompute,
    Technique::SuccessionPatch,
];

/// The representative `Corner` real derivation admits `technique` under —
/// only load-bearing for cells with no `key_scope`, where
/// [`recompute_equivalent`] falls through to the corner match.
fn corner_for(technique: Technique) -> Corner {
    match technique {
        Technique::DeleteInsert => Corner::RecomputeRegion,
        Technique::KeyedFold => Corner::FoldDelta,
        Technique::ColumnScopedMerge => Corner::ColumnMerge,
        Technique::InPlaceUpdate => Corner::FoldDelta,
        Technique::PerGroupRecompute => Corner::ColumnMerge,
        Technique::SuccessionPatch => Corner::FoldDelta,
    }
}

fn key_scope(discovery: KeyDiscovery) -> KeyScope {
    KeyScope {
        keys: vec!["user_id".to_string()],
        from: "upstream".to_string(),
        discovery,
    }
}

/// The `key_scope` shapes to test: no scope, and the three [`KeyDiscovery`]
/// routes. Only `Technique::PerGroupRecompute` cells carry a `key_scope` in
/// real derivation; the matrix still builds a cell for every (technique,
/// shape) pair so the loop's `match` stays genuinely exhaustive over
/// [`Technique`] rather than special-casing one variant.
fn key_scope_shapes() -> Vec<Option<KeyScope>> {
    vec![
        None,
        Some(key_scope(KeyDiscovery::UpstreamKeyed)),
        Some(key_scope(KeyDiscovery::DownstreamGrainOverUpstream)),
        Some(key_scope(KeyDiscovery::EnrichmentKeyed)),
    ]
}

fn cell_for(technique: Technique, scope: Option<KeyScope>) -> PlanCell {
    let mut cell = base_cell(corner_for(technique), technique);
    cell.key_scope = scope;
    cell
}

#[test]
fn every_technique_downgrades_or_needs_nothing_on_trino() {
    let trino_available = StateAvailability::resolve(
        WarehouseTables::Allowed,
        &realisable_state_structures(SqlDialect::Trino),
    );

    for technique in ALL_TECHNIQUES {
        // Exhaustive: a new `Technique` variant added anywhere else makes
        // `ALL_TECHNIQUES` fail to build (its own literal listing must grow
        // too), and this `match` is the second, independent proof — a
        // variant appended to `ALL_TECHNIQUES` alone without extending this
        // arm list is a compile error here.
        match technique {
            Technique::DeleteInsert
            | Technique::KeyedFold
            | Technique::ColumnScopedMerge
            | Technique::InPlaceUpdate
            | Technique::PerGroupRecompute
            | Technique::SuccessionPatch => {}
        }

        // Only `PerGroupRecompute` cells carry a `key_scope` in real
        // derivation (`required_state_structure`'s own doc comment); every
        // other technique is tested once, with no scope.
        let shapes = if technique == Technique::PerGroupRecompute {
            key_scope_shapes()
        } else {
            vec![None]
        };

        for scope in shapes {
            let cell = cell_for(technique, scope.clone());
            let required_before = required_state_structure(&cell);
            let mut cells = vec![cell.clone()];
            resolve_availability(&mut cells, &trino_available);
            let resolved = &cells[0];

            match required_before {
                None => {
                    assert_eq!(
                        resolved.technique, technique,
                        "a cell requiring no state structure must survive Trino's empty \
                         availability unchanged (technique={technique:?}, scope={scope:?})"
                    );
                    assert!(
                        resolved.state_downgrade.is_none(),
                        "a cell requiring no state structure must carry no downgrade \
                         (technique={technique:?}, scope={scope:?})"
                    );
                }
                Some(missing) => {
                    let expected_replacement = recompute_equivalent(&cell);
                    assert_eq!(
                        resolved.technique, expected_replacement,
                        "technique={technique:?} scope={scope:?}: expected the \
                         recompute-equivalent technique"
                    );
                    let downgrade = resolved.state_downgrade.as_ref().unwrap_or_else(|| {
                        panic!(
                            "technique={technique:?} scope={scope:?}: a cell requiring \
                             {missing:?} on Trino (which realises nothing) must carry a \
                             StateDowngrade"
                        )
                    });
                    assert_eq!(downgrade.original, technique);
                    assert_eq!(downgrade.missing, missing);
                    // Reachability half of "claim ⇒ builder": the resolved
                    // cell's OWN requirement must be `None`, so no execution
                    // path reaches a builder looking for `missing`.
                    assert_eq!(
                        required_state_structure(resolved),
                        None,
                        "technique={technique:?} scope={scope:?}: the downgraded cell must \
                         require no state structure of its own, or a run could still reach an \
                         unclaimed builder"
                    );
                }
            }
        }
    }
}

/// Non-vacuity for the test above, and the resolve-late half of the
/// degradation contract: the SAME fixtures under full availability keep
/// their ideal techniques and record no downgrade — so
/// [`every_technique_downgrades_or_needs_nothing_on_trino`] is not passing
/// merely because these fixtures carry no state requirement to begin with.
#[test]
fn the_ideal_plan_survives_resolution_on_trino() {
    let full_availability = StateAvailability::all();

    for technique in ALL_TECHNIQUES {
        let shapes = if technique == Technique::PerGroupRecompute {
            key_scope_shapes()
        } else {
            vec![None]
        };
        for scope in shapes {
            let cell = cell_for(technique, scope.clone());
            if required_state_structure(&cell).is_none() {
                continue;
            }
            let mut cells = vec![cell];
            resolve_availability(&mut cells, &full_availability);
            assert_eq!(
                cells[0].technique, technique,
                "technique={technique:?} scope={scope:?}: full availability must leave the \
                 ideal technique untouched"
            );
            assert!(
                cells[0].state_downgrade.is_none(),
                "technique={technique:?} scope={scope:?}: full availability must record no \
                 downgrade"
            );
        }
    }
}
