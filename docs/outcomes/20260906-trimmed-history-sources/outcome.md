# Outcome: A source whose history is bounded and moving forward refuses or degrades, never silently under-reads

**Created:** 2026-09-06
**Status:** active
**Driver:** outcome loop (`.claude/outcome-backlog`)
**Source:** `docs/research/20260906-bigquery-dogfood.md` §"Trimmed-history sources", §"Trimmed history versus SCD2 lifetime", §Open questions 3 and 4
**Spec anchors:** `docs/specs/sources.md`; `docs/specs/incremental_models.md` §"The equivalence invariant"; `docs/specs/model_properties.md`; `docs/specs/state.md` §"The degradation contract"; `docs/specs/diagnostics.md`

## The outcome

A source can declare that its retained history is **bounded, with the bound moving
forward** as old partitions age out. smelt reasons about that bound the way it reasons
about any other model property: a model whose maintenance needs to read further back than
the source still retains is refused at analysis time with a named code, or degraded with a
recorded downgrade — never silently computed over the history that happens to survive.
Because the bound moves on its own, a model that was admissible last month can stop being
admissible with no change to the code; smelt says so when that happens instead of quietly
returning a smaller answer. The equivalence invariant's quantifier is settled explicitly
for such sources, so `full_refresh(inputs ∈ S)` has one meaning rather than two.

## Success criteria (checkable)

1. **The quantifier is decided.** `docs/specs/incremental_models.md` states whether
   `full_refresh(inputs ∈ S)` for a trimmed source is taken over *retained* history or
   over *all history that ever existed*, with the reasoning. This is the research doc's
   §Open questions 3 and it decides everything downstream — it is settled in the spec
   before any code.
2. **Declaration.** A source declares its retention bound, and the declaration says
   whether the bound is smelt-visible (declared) or merely observed (§Open questions 4 —
   settled in the decision log). Malformed forms refuse with a named `DiagnosticCode` and
   an `examples/broken/` fixture; `diagnostics_catalogue` green.
3. **Reach vs. retention, in the walk.** The comparison between a model's required
   look-back and its source's retained bound is produced by the composition walk in
   `crates/smelt-logical/src/analysis/walk.rs` — not by an ad hoc scan — per the property
   composition walk rule; `cargo test -p smelt-logical --test walk_coverage` green.
4. **Refuse or degrade, never silent.** A model whose reach exceeds the retained bound
   either refuses with a named code or takes a recorded downgrade through the existing
   degradation contract; a test covers each case, and a test asserts no path computes a
   smaller answer without one or the other.
5. **The bound moving is an event.** A source whose bound advances past what an admitted
   model needs is detected on a run and surfaced — the model's admission is re-evaluated
   against the current bound, not the bound at authoring time. A test moves the bound
   forward under a previously-admissible model and asserts the refusal or downgrade fires.
6. **Conformance.** `crates/smelt-maintenance-testkit` gains a trimmed-retention
   `SourceRecipe` whose bound advances between run steps, and
   `cargo test -p smelt-cli --test maintenance_conformance` exercises it against the
   oracle the criterion-1 decision defines. Seeded sample green.
7. **Explain and docs.** `smelt explain` renders the retained bound and the model's
   required reach against it (text and `--json`); a docs-site page documents the
   declaration, the refusal, and the degradation; `cli_docs_coverage` green.
8. **Gates green.** `bash .claude/scripts/verify-phase.sh`, `walk_coverage`,
   `statement_parity`, `execute_parity`, `maintenance_conformance`; ratchets unmoved.

## Out of scope

- The succession grain's interaction with retention (research doc tension 2 — a dimension
  outliving its source's retention). That is a genuine interaction and it is deliberately
  deferred: it cannot be designed before `20260906-scd2-keyed-succession` exists. This
  outcome must leave the quantifier decision (criterion 1) stated clearly enough for that
  work to build on, and nothing more.
- Enforcing or implementing retention — smelt reasons about a bound someone else applies.
- Backfilling history a source no longer retains.
- Any change to the contract lattice's declared points.

- An `--allow-full-refresh` affordance in `smelt-ui` (`run_manager.rs` hardcodes
  `allow_full_refresh: false`, flagged by the phase-6 summary). The UI *refuses* a
  whole-table recompute over a retained source rather than running it silently, so
  criterion 4 is already met; giving the UI a way to license one is a UI affordance, not
  a correctness gap in this outcome.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Settle and spec the equivalence-invariant quantifier for a trimmed source (retained history vs. all history), with reasoning — this decides the rest | done |
| 2 | The rolling-retention declaration: spec + `smelt-core` parse/validation of a moving bound, malformed forms refused with a named `DiagnosticCode` and an `examples/broken/` fixture | done |
| 3 | Reach vs. retention in the composition walk: `analysis/walk.rs` produces the required-look-back vs. retained-bound verdict, no ad hoc scan; `walk_coverage` green | done |
| 4 | Refuse or degrade, never silent: wire the verdict to a named refusal or a recorded downgrade through the degradation contract, plus the no-silent-under-read test | done |
| 5 | The bound moving is an event: admission re-evaluated against the current bound on every run, with a test that advances the bound under a previously-admissible model | done |
| 6 | Whole-table recompute against a trimmed source: a full refresh reaches past every finite bound, so it refuses (or is licensed) rather than silently rebuilding a smaller table | done |
| 7 | Keyed-grain coverage: plumb the driving-source granularity into the run-time retention derivation so a `grain: key` model's plan cannot short-circuit past the retention fold | done |
| 8 | Conformance: a trimmed-retention `SourceRecipe` in `smelt-maintenance-testkit` whose bound advances between run steps, driven through `maintenance_conformance` against the phase-1 oracle | done |
| 9 | Composed-upstream granularity: a `grain: key` model whose sole clocked candidate is an upstream model's composed output still resolves `driving_source_granularity: None` at the run-time retention call site — close that silent skip or record why it cannot be reached | done |
| 10 | Explain and docs: `smelt explain` renders bound vs. required reach (text and `--json`); docs-site page for the declaration, refusal and degradation; `cli_docs_coverage` green | done |
| 11 | Close-out: verify every success criterion's evidence at HEAD, all gates green, ratchets unmoved | planned |

## Decision log

- 2026-09-09 (phase 11 planning): **no reshape.** Row 11 is the last row and every success
  criterion has a named evidence site at HEAD, so nothing serving the criteria is left
  unrowed. Scoping note: the close-out is an *audit against the code*, not against the phase
  summaries — a criterion whose claimed evidence does not actually assert what the criterion
  says gets its test written here, and only a gap needing new production behaviour turns the
  outcome `blocked`.

- 2026-09-09 (phase 10 implementation): shipped the `Retention:` text section and `--json`
  `retention` array; the rendering logic lives in a new `crates/smelt-cli/src/explain/
  retention.rs` module (mirroring `explain/succession.rs`'s own split-out-once-large precedent).
  The residual wiring in `explain.rs`/`commands/explain.rs` (mod decl, one new struct field, two
  new function params, two call sites) is irreducible glue that had to live where the struct and
  function are defined; `.claude/large-file-baseline.txt` was updated for both files with a
  sign-off note (see `phases/10-summary.md` Decisions) since further extraction wasn't possible
  without breaking the struct/function boundary.
- 2026-09-09 (phase 10 planning): **no reshape.** The phase-9 summary confirms rows 10 and 11 are
  unaffected in scope, and no success criterion is unserved by the remaining two rows. Scoping
  note: the explain surface reads `MaintenancePlan::retention_reaches`/`retention_downgrades`
  verbatim and derives nothing (maintenance-plan purity), and renders the bound in **seconds**,
  matching the units `refusal_diag.rs` already prints for `SourceRetentionExceeded` — a new
  interval formatter would be a second spelling of the same quantity. The docs half gains a
  two-sided `SourceRetention*` spec ↔ docs-site sync test, mirroring the existing succession-code
  gate, so the two catalogues cannot drift.
- 2026-09-09 (phase 9 planning): **no reshape; the divergence looks permissive, not silent — the
  phase is scoped as prove-or-close.** Reading the code rather than trusting the phase-7 note:
  the run-time candidate pool is a *subset* of the diagnostics pool, and
  `single_clocked_granularity` is "exactly one element else `None`" with no dedup, so adding the
  composed-upstream candidates can only take `Some → None`, never `None → Some`. A run-time
  `None` (the short-circuit into `locality_refused_plan`) therefore implies a diagnostics `None`
  too, i.e. an Error-severity `KeyedForbidsTimeseries` that `gate_diagnostics` blocks
  pre-execution — the model never runs, so the skipped retention fold is never a silent
  under-read. Two supporting legs: `retention:` is refused without `timeseries:`, so every
  retained ref is necessarily clocked and present in the run-time pool; and `retention:` is a
  source-only declaration, so a composed upstream carries no bound the pool could be missing.
- 2026-09-09 (phase 9 implementation): **leg 3 as stated is FALSE; took the closure branch
  (task 6).** `single_clocked_granularity`'s "exactly one else `None`" rule is not monotone in
  the direction planning assumed: an EMPTY run-time pool (`P → None`) adding exactly one
  composed-upstream candidate (`P′ = P ∪ {g}`) resolves `P′ → Some(g)` — a real `None → Some`
  transition, pinned by `smelt-logical`'s
  `adding_a_candidate_to_an_empty_pool_resolves_an_undecided_granularity`. This is exactly row
  9's named shape: a `grain: key` model with zero declared clocked `sources:` refs whose sole
  clocked candidate is an upstream maintained model's composed output. Separately: for this
  divergence to ever move `retention_reaches`/`retention_downgrades` (the only two fields this
  call site reads), the model would need a directly-referenced `retention:`-bearing source —
  but every `retention:` source is refused without `timeseries:`, so referencing one always
  makes the run-time pool non-empty already, meaning the empty-pool case can never carry
  retention exposure to silently skip. So the closure was taken anyway, for a narrower reason
  than "prevent a silent retention skip": it removes a spurious `Refusal::LocalityNotEstablished`
  this call site's plan carried for an otherwise-admissible composed-only model (inert today,
  since this call site never reads `plan.refusals` — but exactly the kind of latent divergence
  from the diagnostics path this outcome exists to close, and cheap to close now rather than
  leave for a future caller that does read `refusals`). Implementation: `ClampAndLocality`
  (`crates/smelt-runtime/src/propagation/clamp_locality.rs`) now returns its already-computed
  `composed_sources` fixed point; `crate::propagation::composed_source_granularities` exposes it;
  `execute_project` computes it once (gated on any source declaring `retention:`) and threads it
  into `derive_model_retention_plan`, which extends its `SourceFacts`/clocked-granularity pools
  for `grain: key` models exactly the way `smelt-db`'s `maintenance_refs/plan.rs` does. Rows
  10/11 are unaffected — no spec delta, no new refusal shape, `smelt explain --json` was already
  reading the diagnostics path's correct verdict.
  Phase 9 pins all four legs as tests (breaking each premise once to prove sensitivity) and, if
  any leg fails, closes the gap by threading `derive_clamp_and_locality`'s converged
  `composed_sources` map into the call site instead. Rows unchanged.

- 2026-09-09 (phase 8 implement): **the generative pool anchors its schedule a year into the
  future relative to `Utc::now()`.** `smelt-runtime`'s run-time retention admission ages a
  run's window against the REAL wall clock, independent of the schedule's own (often
  synthetic) dates — so a window dated in the past by construction (as every other pool in
  this harness uses, fixed at 2024-01-01) would already read as hundreds of days old today,
  refusing any modest retention bound regardless of the schedule's own internal spacing.
  Anchoring in the future makes `window_age` saturate at zero (an over-future window can never
  have negative age), decoupling admission from calendar drift entirely, while the physical
  trim still uses each step's own `start` as `as_of` so rows depart once `WINDOW_GAP_DAYS`
  (60) exceeds the declared `RETENTION_DAYS` (30) — the two clocks are independent by design,
  not an oversight. See `phases/08-summary.md` for the full reasoning.
- 2026-09-09 (phase 8 planning): **table reshaped — new row 9 for the composed-upstream
  granularity gap the phase-7 summary handed over.** `derive_model_retention_plan`
  (`crates/smelt-runtime/src/execute/retention_admission.rs`) builds its candidate pool from
  `model_file.refs` matched against `source_infos` alone, so a `grain: key` model whose only
  clocked candidate is an upstream *model*'s composed output resolves
  `driving_source_granularity: None` and can still short-circuit into `locality_refused_plan`
  before the retention fold — the same silent-skip class phase 7 closed for declared sources,
  and therefore exactly the silent under-read criterion 4 forbids. It gets a row rather than
  leaving the outcome; it is not folded into phase 8, because phase 8 is a testkit/gate phase
  over declared sources and the fix is production plumbing at a different seam. Rows 9-10 shift
  to 10-11.
- 2026-09-09 (phase 8 planning): **the conformance leg needs no oracle transform and no new
  contract-lattice point.** Verified against `s_tracker.rs`: `s_restricted_oracle_sql`
  materialises the baseline from the tracker's own recorded rows into a temp table
  (`materialize_rows`), never from the physical source relation — so trimming the source
  table leaves the oracle exactly `full_refresh(inputs ∈ S)` over all history ever processed,
  which *is* phase 1's quantifier. Retention is therefore not a declared relaxation of the
  equivalence invariant and mints no lattice point; the leg asserts the unrelaxed invariant.
- 2026-09-09 (phase 8 planning): **the producer's trim rides the existing driver rather than a
  new `ConformanceStep` variant.** Trimming before each run-bearing step, gated on
  `recipe.source.retention.is_some()`, makes the bound advance with the schedule's own clock
  (a rolling interval, per phase 1) and keeps every existing case byte-identical under `None` —
  where a new step variant would churn every `match` over `ConformanceStep` for no added
  fidelity.

- 2026-09-09 (phase 7 planning): **no reshape; the granularity plumbing mirrors the
  diagnostics path, not the propagation path.** Verified against the code rather than the
  phase-5 note: `derive_model_maintenance_plan` calls `establish_locality` (whose structural
  precondition 3 refuses outright on `driving_source_granularity: None`) at
  `crates/smelt-db/src/queries/maintenance/plan.rs`, and the retention fold lives ~120 lines
  further down in `smelt-logical`'s `derive_maintenance_plan_impl` — so a `grain: key` +
  `timeseries:` model genuinely returns `locality_refused_plan` with empty
  `retention_reaches` at run time while the diagnostics path (`maintenance_refs/plan.rs`,
  which resolves the real granularity) admits it. The fix mirrors
  `maintenance_refs/plan.rs`'s *unconditional* `single_clocked_granularity` resolution
  rather than `clamp_locality.rs`'s key-grain-scoped one, because the derivation this run-time
  call must agree with is the diagnostics one; the value is unused for partition grain, so
  unconditional costs nothing. The composed-upstream candidate pool (`model_source_granularities`,
  which this call site cannot see) is the one residual divergence — phase 7 must determine
  reachability and either cover it or hand the phase-8 planner a row, since it is the same
  silent-skip class and may not leave the outcome silently.
- 2026-09-09 (phase 7 planning): the `smelt-ui` `allow_full_refresh` gap the phase-6 summary
  flagged is recorded under **Out of scope** — it is an affordance gap behind a fail-loud
  refusal, not a silent under-read, so no success criterion depends on it.

- 2026-09-09 (phase 6 implement): **gate on `plan.refresh == RefreshStrategy::Incremental`, not `plan.incremental.is_some()`** — the plan's own task 4 named the latter, but it is false for exactly the runs this gate must catch: `build_model_plans`' window-resolution fallback sets `plan.incremental: None` whenever `request.full_refresh` is requested with no explicit `--start`/`--end` (an ordinary `--full-refresh` invocation), collapsing the model to the "full-refresh arm" the way a `materialized_view` model already does by construction. Discovered red (first landing let every full-refresh run through with no gate at all, no error); `plan.refresh` is refresh-strategy-derived and untouched by window resolution, matching that field's own doc comment.
- 2026-09-09 (phase 6 implement): `request.rebuild` (`smelt rebuild`) does not itself license a whole-table recompute — only `request.allow_full_refresh` (`Explicit`) or `force_full_refresh` (`Forced`, smelt-internal) do. An upstream-closure `smelt rebuild` over a model with stored output and a retained source still needs `--allow-full-refresh`, consistent with reusing that one flag as the sole operator override.
- 2026-09-09 (phase 6 implement): audited every `--full-refresh` invocation over `examples/github_activity` (`github_activity_oracle.rs`, `github_activity_replay.rs`) — every one stages a fresh workspace/db, so every one is a first build (auto-licensed); no fixture changes were needed.
- 2026-09-09 (phase 7 implement): confirmed the composed-upstream candidate pool is unreachable
  at this call site (task 5) — `derive_model_retention_plan` builds `source_refs` from
  `model_file.refs` matched against `source_infos` alone (no Salsa `db`/`workspace`), so a ref
  to an upstream *model* never matches and is silently absent, regardless of grain. Left
  uncovered; recorded as a phase-8 candidate row in `phases/07-summary.md` rather than plumbed
  here (adding db/workspace access to this call site is not the "cheap" branch task 5 offered).
- 2026-09-09 (phase 7 implement): the integration fixture's keyed model carries **no** lookback
  construct — `grain: key` forbids window functions and a self-join of its own driving source,
  eliminating both constructs a bounded nonzero reach could come from. Used the identity case
  (`required_lookback: 0`) instead; `retention_refusals_at_age` still refuses on window age
  alone once it exceeds the retained bound, so this is a real instance of the rolling
  re-evaluation over a keyed model, not a weakened substitute for one.
- 2026-09-09 (phase 6 planning): **table reshaped — new row 7 for the keyed-grain
  granularity gap the phase-5 summary surfaced.** `derive_model_retention_plan` passes
  `driving_source_granularity: None`, so a `grain: key` model that declares its own
  `timeseries:` can have `establish_locality` refuse and return `locality_refused_plan`
  (empty `retention_reaches`/`retention_downgrades`) *before* the retention fold in
  `derive/plan.rs` ever runs — verified against
  `crates/smelt-db/src/queries/maintenance/plan.rs`'s early returns, not assumed. That is a
  silent skip of the rolling re-evaluation for a whole model shape, i.e. exactly the silent
  under-read criterion 4 forbids, so it gets a row rather than leaving the outcome. It is
  *not* folded into phase 6, because phase 6's own gate is deliberately built to be immune
  to it (below) and the fix touches a different seam (fact plumbing at the runtime call
  site, not the full-refresh decision). Rows 7-9 shift to 8-10.
- 2026-09-09 (phase 6 planning): **the full-refresh gate reads the model's declared
  `retention:` sources directly, not the maintenance plan's derived reach.** A whole-table
  recompute reaches past *every* finite bound by definition, so there is nothing to derive:
  the input is the set of sources the model reads that declare a bound. This keeps the gate
  total across grains and immune to the phase-5 short-circuit above, and it stays inside
  maintenance-plan purity — reading a declaration is not deriving a composition property,
  and no reach is re-derived anywhere.
- 2026-09-09 (phase 6 planning): **who is refused and who is licensed.** A *user-requested*
  whole-table recompute (`--full-refresh`, `smelt rebuild`) over a model with stored state
  and at least one declared-`retention:` source is refused (`SourceRetentionExceeded`) —
  the stored table is the answer of record over departed history (phase 1's quantifier
  decision), and overwriting it with a partial re-derivation is the unrecoverable move.
  Three cases are licensed instead, each with a recorded downgrade rather than silence:
  a first build (no stored state to destroy — refusing would make the model unbuildable
  forever), a smelt-*forced* full refresh (schema evolution / definition delta — smelt had
  no alternative, so refusing would wedge the model), and an explicit
  `--allow-full-refresh`. Reusing the existing `--allow-full-refresh` rather than minting a
  second flag was chosen because the flag already means exactly "yes, I accept destroying
  and rebuilding this table"; the objection that its existing schema-migration users would
  get a trimmed rebuild they did not ask about is answered by the licensed path being
  *recorded and reported*, never silent. Non-incremental (`table`/`view`) models are out of
  the gate: they are recomputed from scratch every run by construction, hold no answer of
  record accumulated across runs, and refusing them would make them unrunnable.

- 2026-09-09 (phase 5 implement): **`derive_model_retention_plan` must call `maintenance_availability::derive_resolved`, not `smelt_db::queries::maintenance::derive_model_maintenance_plan` directly** — `cargo test -p smelt-runtime --test availability_seam`'s structural gate enforces exactly one call site for the raw derivation in `smelt-runtime`; `StateAvailability::all()` is passed since retention derivation never reads `plan.cells`/availability. Discovered red on first landing, fixed before green.
- 2026-09-09 (phase 5 implement): the runtime fixtures derive their bounded/unbounded reach from a `RANGE BETWEEN ... PRECEDING` window frame, not a `WHERE col >= CURRENT_DATE - INTERVAL '...'` predicate — `CURRENT_DATE` type-checks as `UndeclaredColumn` in the current dialect surface (confirmed against `examples/broken/models/retention_exceeded.sql`, which carries the same diagnostic, just unfiltered by that fixture's own narrower test), which trips `execute_project`'s pre-execution diagnostics gate. Orthogonal to retention; the window-frame pattern is the one phase 3's own unit tests already use.
- 2026-09-09 (phase 5 implement): **known gap, not exercised by this phase's tests** — `derive_model_retention_plan` passes `driving_source_granularity: None`, so a `grain: key` model with its own `timeseries:` block could have its plan derivation short-circuit into `locality_refused_plan` (empty `retention_reaches`) before reaching the retention fold, silently skipping the rolling re-evaluation for that shape. Every `grain: partition` model (this phase's tested shape) is unaffected. Left for phase 6 or a follow-up to confirm scope or plumb the real granularity through.
- 2026-09-09 (phase 5 planning): **the run's required look-back is `derived reach + the age of
  the oldest region the run writes`, measured against the run's own clock** — so the rolling
  re-evaluation needs no new analysis, only the run window plan-time analysis does not have.
  A forward-only run (no `--start`/`--end`) has age zero and is unaffected, which is what keeps
  steady-state maintenance untouched; a backfill of an old region ages into the bound exactly as
  `sources.md` §Semantics 5 describes. The plan therefore carries its own **proof**
  (`MaintenancePlan::retention_reaches`, the bounded per-source reach-vs-retained pair) and the
  rolling step is a pure fold over it in `smelt-logical` — maintenance-plan purity holds: the
  reach is derived once by the walk, never re-derived at run time, and `smelt-runtime` only
  evaluates a pure function of the plan's own data at the run's age.
- 2026-09-09 (phase 5 planning): **table reshaped — a new row 6 for whole-table recompute.**
  A `--full-refresh` (or first build) of a model over a trimmed source reaches past *every*
  finite retained bound and today rebuilds the table from whatever history survives — a silent
  under-read, which is exactly what criterion 4 forbids and what no analysis-time verdict can
  catch (it is a property of the run's shape, not the model's SQL). It was not deferred out of
  the outcome; it gets its own row rather than being folded into phase 5 because refusing it is
  a materially larger behaviour change (it can break an existing `--full-refresh` on
  `examples/github_activity`, whose sources now declare a 45-day bound) and deserves its own
  red-green cycle. Rows 6-8 shift to 7-9.

- 2026-09-09 (phase 4 planning): **`Exceeds` refuses, `UnprovableWithin` takes the recorded
  downgrade, `Within` records nothing.** Three candidate mappings were weighed. Refusing
  every non-`Within` verdict was rejected: any existing model with a `NotDerivable` or
  unbounded reach over a source that later declares `retention:` would break with no code
  change, which is a worse failure than a recorded loss of replayability. Recording a
  warning on every retained source (including honoured ones) was rejected as noise — a
  bound the model provably fits inside is not news, and a diagnostic every run trains
  operators to ignore the one that matters. So: proof present and negative ⇒ refusal
  (`SourceRetentionExceeded`, error, exactly phase 1's wording); proof absent ⇒ recorded
  downgrade (`SourceRetentionDowngraded`, warning) whose material effect is that the
  model's pre-bound region stops being claimed replayable, so a later region recompute
  over it hits the refusal rather than running short; proof present and positive ⇒ nothing.
  Totality of that mapping is itself the criterion-4 "no silent under-read" test.
- 2026-09-09 (phase 4 planning): **the downgrade follows the degradation contract's
  doctrine but not its `StateDowngrade` record.** Derive-ideal-then-downgrade-late,
  recorded, warning-level and explain-visible are all honoured, and the record is surfaced
  alongside `MaintenanceStateDowngraded`; but retention is not a `StateStructure`, and
  reusing `StateDowngrade` would mean inventing a fake structure variant. There is also no
  cheaper-but-still-correct technique to downgrade *to* — under a trimmed source the
  recompute family needs *more* history, not less — so the downgrade narrows the model's
  replayability claim rather than substituting a technique.
- 2026-09-09 (phase 4 planning): the retention map is threaded as a **side channel**
  (`SourceRetentions`, one new `derive_maintenance_plan_with_*` entry point) and the
  downgrade record lands on `MaintenancePlan`, not `PlanCell` — the
  `build_source_referential_integrity` / `build_key_recurrences` precedent, chosen for the
  same reason they were: 67 `ModelInputs` and 47 `PlanCell` literal constructions across
  the workspace stay untouched. Phase table unchanged — the phase-3 summary surfaced no
  work needing a new row (its one open item, where `window_age` comes from, is already
  phase 5's).

- 2026-09-09 (phase 3 implement): **`RetentionVerdict`'s setters and its one
  conversion test live in the new `retention_reach.rs`, not in
  `source_bounds.rs`**, even though `BoundContext` itself is defined there —
  an inherent `impl BoundContext` block does not need to share a file with
  the struct definition in Rust, and `source_bounds.rs` was already pinned at
  its 3367-line ratchet baseline. This kept the unavoidable field-only growth
  in `source_bounds.rs` to +4 lines (the `retentions` field plus its doc
  comment) instead of +41. `crates/smelt-logical/src/rules/incremental.rs`
  still grew by +2 (the two struct-literal fixes task 2 requires — no way to
  add a struct field without touching every literal construction of it).
  Both files were already sitting exactly on their baseline, so any growth
  regresses; reviewer sign-off: both deltas are the minimum the plan's own
  task list requires, baseline updated via
  `.claude/scripts/large-file-check.sh --update`.
- 2026-09-09 (phase 3 implement): test 8 in the phase plan
  ("a_source_absent_from_the_model_gets_no_verdict") assumed
  `derive_model_bounds` omits a source the model's SQL never reads. In fact
  `derive_model_bounds`'s existing whole-text top-up (`source_bounds.rs`,
  predating this phase) backfills **every** source present in
  `ctx.source_partition_cols` with at least `Bounded{0,0}` when the walk
  itself doesn't find it as a FROM leaf — so a source *registered* in ctx is
  never actually absent from the bounds map. The test instead covers the
  case the map genuinely omits: a source with a declared `retention:` but no
  corresponding `ctx.source_partition_cols` entry at all (not a timeseries
  source in this model's context). Verified against the real fallback
  behaviour before rewriting the test, not assumed.
- 2026-09-09 (phase 3 planning): **the verdict is a fold over the walk's output, in a new
  module, not a new transfer function.** The required look-back the criterion names is
  already the composition walk's own product (`derive_model_bounds`'s `BoundResult`, walked
  via `QueryTree` in `analysis/source_bounds.rs` — `analysis/walk.rs` named in criterion 3
  is now the `analysis/walk/` module). So the comparison needs no second walk: it is a pure
  map over that verdict against `BoundContext`'s declared retention, and it inherits series
  composition (stacked frames *add*) for free — a test pins exactly that, since a whole-text
  max-merge would under-derive the reach and wrongly admit. It lands in a new
  `analysis/retention_reach.rs` rather than inside `source_bounds.rs`, which sits at its
  3367-line large-file ratchet baseline. The verdict also carries a `window_age` term
  (`required_lookback = before + window_age`) so the rolling re-evaluation phase 5 needs is
  built in from the start rather than retrofitted. Phase table unchanged — the phase-2
  summary surfaced no work needing a new row.

- 2026-09-09 (phase 2 implement): **an inert retention in `examples/timeseries` was
  removed, not made well-formed.** `examples/timeseries/models/sources/raw/events.yml`
  declared `retention: '400 days'` on a source with no `timeseries:` — exactly the shape
  this phase refuses. Rather than adding a `timeseries:` block to make it parse, the
  `retention:` line was deleted: the source is deliberately an unclocked lookup
  (`watermark:` + `unique_key:` only), and nothing in the workspace reads its retention
  value. `crates/smelt-core/src/sources.rs` also grew past its large-file-ratchet baseline
  (1297 → 1336); reviewer sign-off: three cohesive error variants plus ~20 lines of
  validation in the file that already single-owns source YAML parsing, baseline updated
  via `.claude/scripts/large-file-check.sh --update`.
- 2026-09-08 (phase 2 planning): **a malformed rolling bound is refused, and an inert one
  counts as malformed** — phase 2 refuses `retention:` when the interval is unparseable,
  when it is zero, and when the source declares no `timeseries:`. Reasoning: the first is
  ordinary fail-loud parsing (today it escapes as an opaque serde `YamlParse` naming the
  retired `data_latency` key); the last two are the same failure this outcome exists to
  prevent, one level up — a bound with no clock has no reach to be compared against, so
  accepting it would silently license every replay it was written to forbid. No new
  `DiagnosticCode`: `MalformedSource` already names the retention clause in
  `sources.md` §"Diagnostic codes". Phase table otherwise unchanged — the phase-1 summary
  surfaced no work needing a new row.

- 2026-09-08 (phase 1 implement): **quantifier settled: over all history ever processed,
  never narrowed by retention.** `docs/specs/incremental_models.md` §"The equivalence
  invariant" now states it directly: a partition that ages out of `retention:` was still
  scanned, so it stays in `S` and the stored table remains the answer of record over it —
  retention narrows the *executable* `full_refresh` oracle's replayable region, never the
  quantifier. A recompute reaching past the bound is refused (`SourceRetentionExceeded`)
  rather than run to produce a smaller answer. Reasoning: this is the existing
  replayability split applied to a bound that moves, and it is what keeps the feature
  small — only a model whose reach exceeds the bound, or an explicit backfill, is ever
  affected; steady-state forward-only maintenance never is.
- 2026-09-08 (phase 1 implement): **retention is declared, verified, never observed** —
  `docs/specs/sources.md`'s `retention:` row and §Semantics 5 now say so explicitly.
  Reasoning: `retention:` is a narrowing world-fact under the existing trust rule (it
  licenses smelt to stop trusting replay over a region), so it follows the same
  discipline as every other narrowing declaration — trusted only paired with a
  verification mechanism, never discovered by probing the warehouse, with over-claiming
  (longer than the producer actually keeps) as the unsafe direction the mechanism must
  catch. `retention:` is also **rolling** (anchored to the current run, advancing on its
  own) rather than a fixed calendar date, because that is the physical fact real
  warehouses expose (BigQuery `partition_expiration_days`, Delta's retention window).
- 2026-09-08 (phase 1 implement): reconciled `examples/github_activity`'s inert
  `retention: '90 days'` to the loader's real 45-day `partition_expiration_days` on both
  `github_events.yml` and `github_events_arrival.yml`, per the spine's findings handoff;
  new red-then-green test
  `crates/smelt-cli/tests/github_activity_loader.rs::source_yaml_retention_matches_the_loader_expiration`
  guards the two staying in sync.
- 2026-09-08 (phase 1 planning): **table reshaped from the placeholder** into rows 2-8, one
  per remaining success criterion, ordered declaration → walk verdict → refuse/degrade →
  bound-movement → conformance → explain/docs → close-out. Nothing left the outcome; the
  spine's handoff requirement (reconciling the example's inert `retention: '90 days'` against
  the loader's real 45-day `partition_expiration_days`) is folded into phase 1 rather than
  deferred, since it is a doc-level fact the quantifier decision settles.

- 2026-09-08 (bigquery-dogfood-spine phase 15): **the interim findings handoff now
  exists** at `docs/handoffs/2026-09-08-github-activity-findings.md`, covering the
  DuckDB half only ("Requirements handed to `20260906-trimmed-history-sources`" section) —
  the loader's 45-day `partition_expiration_days` bound and its derivation, and the
  inert, now-corrected-comment-but-still-inconsistent `retention: '90 days'` field on both
  `examples/github_activity/models/sources/raw/github_events{,_arrival}.yml` that this
  outcome must reconcile. The live-BigQuery half lands in that outcome's phase 16.
- 2026-09-06 (scaffold): **deliberately short**, for the same reason as
  `20260906-external-dag-steps` — the declaration's shape depends on what the spine's
  loader actually retains and on whether that retention is a smelt-visible declaration or
  an observed property (§Open questions 4). The phase-1 planner completes the table from
  the spine's findings handoff.
- 2026-09-06 (scaffold): criterion 1 is ordered first on purpose. The quantifier question
  is not a detail — under "retained history" a trimmed source is ordinary and the feature
  is small; under "all history that ever existed" every maintained model over a trimmed
  source is permanently degraded. The answer changes the size of this outcome by an order
  of magnitude, so nothing else is planned until it is written down.
- 2026-09-06 (scaffold): tension 2 (SCD2 lifetime beyond source retention) is out of scope
  here but is *not* dropped — it is named in Out of scope with the dependency that blocks
  it, so the succession work inherits it rather than losing it.

## Blocked

(none)
