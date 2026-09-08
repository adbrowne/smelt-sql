# Phase 04 — Refuse or degrade, never silent

## Objective

Give phase 3's `RetentionVerdict` an action. A model whose derived reach is *proven* to
exceed a source's declared `retention:` refuses at plan time with a named
`SourceRetentionExceeded`; one whose reach cannot be proven to fit takes a **recorded**
downgrade (`SourceRetentionDowngraded`, warning) rather than being admitted as if it fit.
Advances success criteria 4 and 8; leaves criterion 5's rolling re-evaluation (the
`window_age` term) to phase 5.

## Spec delta (spec-first — the implement step makes these edits first)

- `docs/specs/sources.md` §Semantics 5 ("Retention refusal"): the *unprovable* case is
  stated alongside the exceeding case — a reach the unified derivation cannot bound against
  a declared `retention:` is not admitted silently; the plan records a downgrade naming the
  source, the retained bound and the reason, and the model's pre-bound region stops being
  claimed replayable. Add a `SourceRetentionDowngraded` row (Warning, plan derivation) to
  §"Diagnostic codes"; drop `SourceRetentionExceeded` from the "remain unbuilt" sentence at
  the end of the spec (~L440).
- `docs/specs/model_properties.md` §"Reach versus retained history": add the action half —
  the verdict→outcome mapping (`Within`/`NoDeclaredBound` → nothing recorded;
  `Exceeds` → refusal; `UnprovableWithin` → recorded downgrade), and the totality rule that
  no other outcome exists.
- `docs/specs/diagnostics.md`: catalogue both codes (`diagnostics_catalogue` gate).

## Tests (red → green)

`crates/smelt-logical/tests/retention_admission.rs` (new):
1. `exceeding_reach_refuses_with_source_retention_exceeded` — 30-day lookback over a source
   declaring 7 days ⇒ `Refusal::SourceRetentionExceeded` naming source, required, retained.
2. `reach_within_retention_records_nothing` — 1-day lookback, 7-day retention ⇒ no refusal,
   no downgrade (the no-noise case: an honoured bound is silent by design).
3. `unprovable_reach_records_a_retention_downgrade` — unbounded/cumulative read over a
   retained source ⇒ one `RetentionDowngrade` carrying the `UnprovableReason`, no refusal.
4. `no_declared_retention_leaves_the_plan_unchanged` — same model derived through the new
   entry point with an empty retentions map equals `derive_maintenance_plan`'s plan.
5. `every_retention_verdict_maps_to_a_refusal_a_downgrade_or_an_admitted_fit` — table over
   all four `RetentionVerdict` shapes against `retention_outcomes`: exactly `Within` and
   `NoDeclaredBound` record neither. This is the no-silent-under-read gate.

`crates/smelt-db/src/queries/maintenance/tests.rs` (or its sibling test module):
6. `retention_refusal_maps_to_source_retention_exceeded_error` — projection + code +
   `Severity::Error`, message names both intervals.
7. `retention_downgrade_surfaces_as_a_warning_never_an_error` — `SourceRetentionDowngraded`
   at Warning, and the model still derives cells (a downgrade is not a refusal).

`crates/smelt-cli/tests/example_diagnostics.rs` fixture leg:
8. `broken_retention_exceeded_example_reports_the_named_code` — new
   `examples/broken/` model (source `retention: '7 days'`, model with a 30-day window)
   reports `SourceRetentionExceeded` and nothing else new.

## Tasks

1. Spec edits above (sources.md, model_properties.md, diagnostics.md).
2. New `crates/smelt-logical/src/maintenance/retention.rs`: `SourceRetentions`
   (bare source name → `DataLatency`), `RetentionDowngrade { source, retained, reason }`,
   and the pure total mapping `retention_outcomes(&HashMap<String, RetentionVerdict>)
   -> (Vec<Refusal>, Vec<RetentionDowngrade>)`. Register in `maintenance/mod.rs`.
3. `Refusal::SourceRetentionExceeded { source, required_lookback_secs, retained_secs }` in
   `maintenance/refusal.rs`; `MaintenancePlan.retention_downgrades: Vec<RetentionDowngrade>`
   in `maintenance/plan.rs` (plan-level, not per-cell — the verdict is a property of the
   model against a source, like `fingerprint_projections`, and `MaintenancePlan: Default`
   keeps all 47 `PlanCell` literals untouched).
4. `derive/plan.rs`: extend `derive_maintenance_plan_impl` with a `retentions:
   &SourceRetentions` parameter; add one public entry point taking both side channels
   (referential integrity + retentions), the existing two delegating with an empty map.
   Build the `BoundContext` retentions via `with_source_retention`, call
   `derive_retention_verdicts(sql, ctx, Seconds(0))` — `window_age` stays 0 with a doc
   comment naming phase 5 as its owner — and fold the outcomes onto the plan.
5. `plan_helpers.rs`: `build_source_retentions(refs)` following the
   `build_source_referential_integrity` precedent; thread it through
   `crates/smelt-db/src/queries/maintenance/plan.rs`'s derivation call.
6. `diagnostics_types/mod.rs`: `SourceRetentionExceeded` + `SourceRetentionDowngraded`
   variants; `refusal_diag.rs` + `diagnostics.rs`: the `MaintenanceRefusal` projection arm,
   the message, and a downgrade projection surfaced at Warning.
7. `examples/broken/` fixture for test 8.
8. If `.claude/large-file-baseline.txt` regresses (`diagnostics_types/mod.rs` sits exactly
   at 1211), re-run `--update` and record the sign-off in the phase summary.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --quiet` (incl. the new `retention_admission` suite)
- `cargo test -p smelt-logical --test walk_coverage --quiet`
- `cargo test -p smelt-db --test integration diagnostics_catalogue --quiet`
- `cargo test -p smelt-cli --test example_diagnostics --quiet`
- `bash .claude/scripts/large-file-check.sh`

## Commit message

`feat(maintenance): refuse or record a downgrade when a model's reach meets a source's retained bound`
