use super::*;
use crate::maintenance::Trigger;

fn suppressed() -> WriteSuppression {
    WriteSuppression::Suppressed {
        compared_columns: vec!["tier".to_string()],
    }
}

fn unconditional() -> WriteSuppression {
    WriteSuppression::Unconditional {
        why: "column(s) notes are not proven comparable".to_string(),
    }
}

#[test]
fn steady_state_trigger_prefers_suppression_when_admitted() {
    let trigger = Trigger::UpstreamMutation {
        source: "sources.users".to_string(),
    };
    let (resolved, reason) = resolve_write_variant(
        &suppressed(),
        &trigger,
        false,
        &EffectiveOverride::default(),
    )
    .expect("no pin — never refuses");
    assert_eq!(resolved, suppressed());
    assert_eq!(reason, VariantReason::SteadyStatePreference);
}

#[test]
fn backfill_trigger_admits_but_does_not_prefer_suppression() {
    // First build (no prior state) routes through `Trigger::Backfill` —
    // admitted (the proof still holds) but not preferred: resolves
    // unconditional by default.
    let (resolved, reason) = resolve_write_variant(
        &suppressed(),
        &Trigger::Backfill,
        false,
        &EffectiveOverride::default(),
    )
    .expect("no pin — never refuses");
    assert!(matches!(resolved, WriteSuppression::Unconditional { .. }));
    assert_eq!(reason, VariantReason::FirstBuildPosture);
}

#[test]
fn ledger_catch_up_admits_but_does_not_prefer_suppression_even_on_steady_state_trigger() {
    // A definition-change backfill cell (`ledger_catch_up: true`) has no
    // prior state for this group regardless of trigger kind.
    let trigger = Trigger::UpstreamMutation {
        source: "sources.users".to_string(),
    };
    let (resolved, reason) =
        resolve_write_variant(&suppressed(), &trigger, true, &EffectiveOverride::default())
            .expect("no pin — never refuses");
    assert!(matches!(resolved, WriteSuppression::Unconditional { .. }));
    assert_eq!(reason, VariantReason::FirstBuildPosture);
}

#[test]
fn not_admitted_passes_through_unchanged_regardless_of_trigger() {
    let trigger = Trigger::UpstreamMutation {
        source: "sources.users".to_string(),
    };
    let (resolved_steady, reason_steady) = resolve_write_variant(
        &unconditional(),
        &trigger,
        false,
        &EffectiveOverride::default(),
    )
    .expect("no pin — never refuses");
    assert_eq!(resolved_steady, unconditional());
    assert_eq!(reason_steady, VariantReason::NotAdmitted);

    let (resolved_backfill, reason_backfill) = resolve_write_variant(
        &unconditional(),
        &Trigger::Backfill,
        false,
        &EffectiveOverride::default(),
    )
    .expect("no pin — never refuses");
    assert_eq!(resolved_backfill, unconditional());
    assert_eq!(reason_backfill, VariantReason::NotAdmitted);
}

#[test]
fn new_data_trigger_with_prior_state_prefers_suppression() {
    let trigger = Trigger::NewData {
        source: "sources.users".to_string(),
    };
    let (resolved, reason) = resolve_write_variant(
        &suppressed(),
        &trigger,
        false,
        &EffectiveOverride::default(),
    )
    .expect("no pin — never refuses");
    assert_eq!(resolved, suppressed());
    assert_eq!(reason, VariantReason::SteadyStatePreference);
}

// --- Phase G1: `cells[].technique: suppress|unconditional` pins ---
// (docs/plans/20260715-composed-axes-conditional-maintenance.md Phase
// G1's required TDD test: "`cells[].technique` pins either way and an
// inadmissible pin refuses (never falls back silently)", for the
// write-suppression dimension.)

#[test]
fn technique_suppress_pin_forces_suppression_on_for_a_first_build_cell() {
    // Structurally, a first-build/backfill trigger defaults to
    // unconditional (`backfill_trigger_admits_but_does_not_prefer_
    // suppression` above) — a hard `technique: suppress` pin overrides
    // that default.
    let overrides = EffectiveOverride {
        prefer: None,
        technique: Some(CellTechnique::Suppress),
    };
    let (resolved, reason) =
        resolve_write_variant(&suppressed(), &Trigger::Backfill, false, &overrides)
            .expect("suppression is admitted (proof holds) — the pin must be honoured");
    assert_eq!(resolved, suppressed());
    assert_eq!(reason, VariantReason::Overridden);
}

#[test]
fn technique_unconditional_pin_forces_suppression_off_for_a_steady_state_cell() {
    // Structurally, a steady-state trigger over prior state prefers
    // suppression (`steady_state_trigger_prefers_suppression_when_
    // admitted` above) — a hard `technique: unconditional` pin
    // overrides that default.
    let trigger = Trigger::UpstreamMutation {
        source: "sources.users".to_string(),
    };
    let overrides = EffectiveOverride {
        prefer: None,
        technique: Some(CellTechnique::Unconditional),
    };
    let (resolved, reason) = resolve_write_variant(&suppressed(), &trigger, false, &overrides)
        .expect("`unconditional` is always admissible — never refuses");
    assert!(matches!(resolved, WriteSuppression::Unconditional { .. }));
    assert_eq!(reason, VariantReason::Overridden);
}

#[test]
fn technique_suppress_pin_refuses_when_the_suppression_proof_itself_refused() {
    // The write-suppression proof (P2/P3) refused for this cell
    // (`unconditional()` — e.g. an incomparable column) — a
    // `technique: suppress` pin cannot force suppression on over a
    // genuine admission failure; it must refuse loudly, never silently
    // fall back to the unconditional matched arm.
    let trigger = Trigger::UpstreamMutation {
        source: "sources.users".to_string(),
    };
    let overrides = EffectiveOverride {
        prefer: None,
        technique: Some(CellTechnique::Suppress),
    };
    let err = resolve_write_variant(&unconditional(), &trigger, false, &overrides)
        .expect_err("pinning suppression over a refused P2/P3 proof must refuse");
    assert_eq!(
        err.pinned,
        PinnedRequest::Technique(CellTechnique::Suppress)
    );
    assert!(err.why.contains("proof itself refused"));
}

#[test]
fn prefer_suppress_soft_bias_overrides_first_build_default_without_refusing() {
    // A soft `prefer: suppress` bias, unlike the hard pin, still never
    // refuses — but it does override the structural first-build
    // default when the variant is admitted.
    let overrides = EffectiveOverride {
        prefer: Some(TechniquePreference::Suppress),
        technique: None,
    };
    let (resolved, reason) =
        resolve_write_variant(&suppressed(), &Trigger::Backfill, false, &overrides)
            .expect("soft bias never refuses");
    assert_eq!(resolved, suppressed());
    assert_eq!(reason, VariantReason::Overridden);
}

#[test]
fn prefer_suppress_soft_bias_falls_back_silently_when_not_admitted() {
    // Unlike the hard pin, a soft `prefer: suppress` bias falls back
    // silently (no refusal) when the write-suppression proof itself
    // refused — "soft" means it only nudges among what IS resolvable.
    let trigger = Trigger::UpstreamMutation {
        source: "sources.users".to_string(),
    };
    let overrides = EffectiveOverride {
        prefer: Some(TechniquePreference::Suppress),
        technique: None,
    };
    let (resolved, reason) = resolve_write_variant(&unconditional(), &trigger, false, &overrides)
        .expect("soft bias never refuses");
    assert_eq!(resolved, unconditional());
    assert_eq!(reason, VariantReason::NotAdmitted);
}
