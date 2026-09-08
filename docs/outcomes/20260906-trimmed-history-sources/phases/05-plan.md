# Phase 05 plan — The bound moving is an event

## Objective

Make the reach-vs-retention admission a **run-time** question, not an authoring-time one:
the required look-back a run must prove is `derived reach + the age of the oldest region the
run writes`, measured against the run's own clock, so a region that was inside the bound last
month is refused today with no change to the model. Advances success criterion 5, and closes
criterion 4's run-side half (no run path computes a smaller answer silently).

## Spec delta

`docs/specs/sources.md` §Semantics 5 ("Retention refusal") already commits to the rolling
re-evaluation; it does not say what the run's age is measured from. Add two sentences there
(and mirror one in `docs/specs/model_properties.md` §"Reach versus retained history"):

- The quantity compared against `retention:` on a run is **the model's derived reach plus the
  age of the oldest region the run writes**, aged against the run's own clock. A forward-only
  run (no explicit window) has age zero, so steady-state maintenance is never affected; a
  backfill of an old region ages into the bound.
- The refusal fires **before any statement executes** for that model, naming the source, the
  required look-back, the retained bound, and the run window that produced the age — a refused
  model leaves its stored output untouched rather than overwriting it with a partial
  re-derivation.

Do not restate the `Exceeds`/`UnprovableWithin`/`Within` mapping — phase 4 owns it.

## Tests

Red-green, in this order.

1. `smelt-logical` `retention_reaches_carry_the_bounded_proof` — `derive_maintenance_plan_with_
   referential_integrity_and_retentions` populates `MaintenancePlan::retention_reaches` with one
   entry per bounded verdict (`Within` and `Exceeds` alike), and none for `UnprovableWithin` /
   an undeclared bound.
2. `smelt-logical` `an_admissible_reach_refuses_once_the_window_ages_past_the_bound` — the
   criterion's own test: one `RetentionReach { required_lookback: 7d, retained: 45d }`,
   `retention_refusals_at_age(.., ZERO)` is empty, `.. Seconds::days(60)` yields exactly one
   `Refusal::SourceRetentionExceeded` reporting `required_lookback = 67d`.
3. `smelt-logical` `age_never_rescues_an_already_exceeding_reach` — a reach already past the
   bound at age zero still refuses at every larger age (monotone; no age arithmetic can flip a
   refusal back to silence).
4. `smelt-logical` `run_window_age_is_zero_for_a_window_at_or_after_now` — the age helper
   saturates at zero rather than going negative for a forward-dated window.
5. `smelt-logical` `retention_reaches_survive_a_plan_round_trip_unchanged` — the rolling fold
   reads only `retention_reaches`; it never re-walks the SQL (maintenance-plan purity).
6. `smelt-runtime` `a_backfill_window_older_than_retention_refuses_before_any_statement` — a
   model over a `retention: '45 days'` source, run with a window starting 90 days before the
   run clock, fails with `SourceRetentionExceeded` naming the source, and the model's target
   table is untouched.
7. `smelt-runtime` `a_forward_only_run_over_the_same_model_still_succeeds` — the same project
   with no explicit window runs green (age zero; steady state unaffected). Guards against the
   check turning into a blanket refusal on every retained source.
8. `smelt-runtime` `the_retention_downgrade_is_reported_once_per_run` — an `UnprovableWithin`
   model surfaces its `RetentionDowngrade` through the run reporter as a warning, not silence.

## Tasks

1. Add `RetentionReach { source, required_lookback, retained }` to
   `crates/smelt-logical/src/maintenance/retention.rs` and
   `MaintenancePlan::retention_reaches: Vec<RetentionReach>` to `maintenance/plan.rs` (fix the
   ~6 literal constructions in `plan.rs` + `succession.rs`).
2. Have `retention_outcomes` also return the bounded reaches (or add a sibling collector) and
   fold them onto the plan in `maintenance/derive/plan.rs`, replacing the `Seconds::ZERO`
   comment that names this phase.
3. Add pure `retention_refusals_at_age(&[RetentionReach], window_age: Seconds) -> Vec<Refusal>`
   and `run_window_age(window_start: NaiveDate, now: NaiveDate) -> Seconds` (saturating) to the
   same module — the single owner of the rolling arithmetic.
4. New `crates/smelt-runtime/src/execute/retention_admission.rs`: given a model's already-derived
   plan, the run's resolved window start, and the run clock (`run_start` in
   `execute/project/mod.rs`, never `Utc::now()` inline), evaluate task 3's fold and return a
   refusal error naming source + required look-back + retained bound + window.
5. Call it from `execute/project`'s per-model path **before** any statement is emitted or
   executed for that model, and report each `plan.retention_downgrades` entry as a reporter
   warning on the same pass.
6. Fixtures for tests 6-8 under the runtime test tree (a `retention: '45 days'` source plus a
   7-day-lookback model and an unbounded-reach model).
7. Run `bash .claude/scripts/large-file-check.sh`; if `plan.rs` or `execute/project/mod.rs`
   regresses, `--update` with a one-line sign-off in the phase summary.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test walk_coverage`
- `cargo test -p smelt-runtime --test execute_parity --test statement_parity --test availability_seam`
- `cargo test -p smelt-db --test integration diagnostics_catalogue`
- `cargo test -p smelt-cli --test example_diagnostics`
- `bash .claude/scripts/large-file-check.sh`

## Commit message

`feat(maintenance): re-evaluate a source's retained bound against the run's own window age`
