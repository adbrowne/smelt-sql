# Phase 07 plan — Keyed-grain coverage for the run-time retention re-evaluation

## Objective

`derive_model_retention_plan` (`crates/smelt-runtime/src/execute/retention_admission.rs`)
passes `driving_source_granularity: None`, so a `grain: key` model that also declares a
`timeseries:` block fails `establish_locality`'s granularity-equality precondition and
returns `locality_refused_plan` — empty `retention_reaches`/`retention_downgrades` —
*before* the retention fold in `smelt-logical/src/maintenance/derive/plan.rs` ever runs.
The run-time rolling re-evaluation is therefore skipped for that whole model shape, while
the plan-time/diagnostics path (which resolves the real granularity) admits the model:
a silent under-read. This phase resolves the granularity at the run-time call site the
same way, so the two derivations agree. Advances criteria 4 and 5.

## Spec delta

None. `docs/specs/sources.md` §Semantics 5 already states the rolling re-evaluation
without scoping it to a grain; this is an implementation shape gap, not a behaviour
change to the specified surface. If the implementer finds any spec or docs wording that
scopes retention re-evaluation to partition-grain models, correct it to grain-independent
as the first commit-step (spec-first).

## Tests

Unit (`crates/smelt-runtime/src/execute/retention_admission.rs` tests module — build the
`ModelFile`/`SourceInfo` inputs directly, no backend needed):

1. `keyed_model_with_timeseries_reaches_the_retention_fold` — a `grain: key` +
   `timeseries:` model over one clocked, `retention:`-bearing source yields a plan with
   non-empty `retention_reaches` and **no** `Refusal::LocalityNotEstablished`. RED today
   (empty reaches, locality refusal).
2. `two_clocked_sources_leave_the_granularity_undecided` — with two referenced clocked
   sources of different granularities the resolution is `None`, i.e. the shared
   `single_clocked_granularity` rule is used rather than a bespoke re-derivation.
3. `partition_grain_derivation_is_unchanged` — a `grain: partition` model derives the same
   `retention_reaches` as before the change (regression pin; GREEN before and after).

Integration (`crates/smelt-runtime/tests/retention_admission.rs`, extending the existing
fixture with a keyed model over the same 45-day-retention `sources.events`):

4. `a_keyed_model_backfill_older_than_retention_refuses` — a `grain: key` model whose reach
   ages past the bound refuses before any statement executes. RED today (runs silently).
5. `a_forward_only_run_over_the_keyed_model_still_succeeds` — age zero, no false refusal
   from the newly-plumbed granularity.

Fixture constraint: the keyed model must actually clear `establish_locality`, so route 1
(key-embedded) is the cheapest shape — `partition_column` is itself a `unique_key:` column,
the driving source declares its own `partition_column`, and the reach comes from a bounded
`RANGE ... PRECEDING` frame (not a `CURRENT_DATE` predicate — see phase 5's decision log).
If a locality-admissible keyed fixture carrying a bounded lookback proves not constructible,
keep tests 1-3 as the phase's evidence, record precisely what blocked the integration
fixture in the summary, and do **not** claim criterion 5 covers keyed grain end-to-end.

## Tasks

1. Correct `derive_model_retention_plan`'s doc comment — it currently asserts that
   `driving_source_granularity` never reaches the retention fold, which is false and is the
   reason the gap shipped.
2. Resolve the granularity in `derive_model_retention_plan` from the model's own source refs
   (`build_succession_source_refs`'s `(ref, Option<SourceInfo>)` pairs are already built
   there) via `smelt_logical::maintenance::locality::single_clocked_granularity`, mirroring
   `crates/smelt-db/src/maintenance_refs/plan.rs`'s unconditional form — the diagnostics
   path this derivation must agree with — not `clamp_locality.rs`'s key-grain-scoped form.
3. Thread it through the `crate::maintenance_availability::derive_resolved` call (the one
   permitted seam; `availability_seam`'s structural gate stays green).
4. Add the keyed model to the integration fixture's staged project and write tests 4-5.
5. Check the composed-upstream case: a keyed model driven by an upstream *model* rather
   than a declared source contributes no candidate here, while `maintenance_refs/plan.rs`
   extends its pool with `model_source_granularities`. Determine whether that divergence is
   reachable at this call site; cover it if it is cheap, otherwise record it in the summary
   with enough detail for the phase-8 planner to add a row (it is the same silent-skip
   class, so it does not leave the outcome silently).
6. Update `.claude/large-file-baseline.txt` only if a touched file regresses, with a
   sign-off line in the summary.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test retention_admission --test retention_full_refresh --test availability_seam --test execute_parity --test statement_parity`
- `cargo test -p smelt-logical --test walk_coverage`
- `cargo test -p smelt-cli --test example_diagnostics`
- `bash .claude/scripts/large-file-check.sh`

## Commit message

`fix(maintenance): resolve the driving-source granularity in the run-time retention derivation`
