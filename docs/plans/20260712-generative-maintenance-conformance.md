# Plan: Generative Maintenance Conformance Harness

**Date**: 2026-07-12
**Spec**: [`docs/specs/maintenance_plan.md`](../specs/maintenance_plan.md) — §"The equivalence invariant" is the correctness oracle
**Design**: [`docs/research/20260711-generative-maintenance-conformance.md`](../research/20260711-generative-maintenance-conformance.md) (the approved change description — this plan does not restate it)
**Spec diff**: none (test infrastructure; the spec's invariant is unchanged — this plan builds its generative regression net)
**Tracking PR / branch**: `worktree-incremental_2`
**Docs**: code-only (no user-facing surface; Phase 11 updates repo-internal spec References and `CLAUDE.md` gate listings)

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read the design doc `docs/research/20260711-generative-maintenance-conformance.md` and `docs/specs/maintenance_plan.md` §"The equivalence invariant", §"Per-cell admission", §"The reconciliation ledger" — the design doc is the architecture; the spec is the correctness oracle. Do not re-open settled design decisions (oracle modes, numeric discipline, disposition table).
2. Confirm you are on branch `worktree-incremental_2`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule — in particular, if a generated case exposes a REAL production divergence from the equivalence invariant, do NOT weaken the oracle: pin the shrunk recipe as an explicit failing regression test, record it in the divergence registry as `KnownBug`, file it in this plan's "Deferred during implementation", and continue (a discovered production bug is a deliverable, not a blocker — fixing it is its own red-green change outside this plan unless trivially mechanical).
- A design assumption turns out wrong (update the research doc first, then continue).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**
- Real-execution fixtures: this plan's "real fixture" standard is a *staged generated project driven through the real `execute_project` pipeline on real DuckDB* (the Link-C rule — never `run_incremental_sequence`, never hand-injected `WHERE`). `examples/` fixtures appear only where a phase explicitly names them.
- Red-green TDD: failing test before any implementation.
- Verification gate is `bash .claude/scripts/verify-phase.sh` — one call, failures-only output.
- `DUCKDB_LIB_DIR` + `LD_LIBRARY_PATH` must be set (see `CLAUDE.md`); if unset, stop and report — never let DuckDB-gated tests silently skip.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking branch.
- Don't widen scope: a phase may not reach into a later phase's scope.
- Honor architectural invariants from `CLAUDE.md` — especially run-pipeline parity (only `execute_project` drives runs), maintenance-plan purity (never re-derive plan data in the harness; consume `derive_model_maintenance_plan`), and the fail-loud gates.
- **Timeless-oracle rule.** Phase vocabulary lives in this plan file only. The Phase-11 spec/`CLAUDE.md` edits describe the gates as if they have always existed.

---

## Context

`maintenance_plan.md` §"The equivalence invariant" promises `incremental_state(S) == full_refresh(inputs ∈ S)` for every maintained model under any valid run sequence. Its executable regression net today covers a hand-enumerated model catalogue only; the design doc (§2) itemizes the seven gaps. This plan adds the generative axis: a `Strategy<ModelRecipe>` over the existing `smelt-maintenance-testkit` harness, an S-tracked dual-mode full-refresh oracle, plan-claim probes, simulated change feeds, generated DAG propagation checks, and the graduation/retirement of the disposable probe layer — per the design doc's §3–§12, which this plan executes phase-for-phase.

## Scope

### In scope (design coverage)
- Design §4: `ModelRecipe` typed generation + rendering (model SQL, source YAMLs, `smelt.yml`, oracle SQL)
- Design §3/§8: admitted/refused verdict protocol, fail-loud refusal assertions, over-refusal ledger, generator-health reachability gate
- Design §5: schema-generic data + schedule generation (new step kinds, numeric discipline, boundary placement, simulated change feed)
- Design §6: S-tracker + S-restricted and settled-point oracle modes, keyed carve-outs
- Design §7: plan-claim probes (clamp consistency, write-window containment, sensitivity, ledger incl. persisted `.smelt/reconciliation.json`, interchangeability pins, order-determinacy)
- Design §9: generated 2–3 node DAGs — propagation sufficiency, backward resolution, payload-leak family
- Design §10: standing gate `cargo test -p smelt-cli --test maintenance_conformance`, env-scaled depth, divergence registry, soak wiring
- Design §11: catalogue → pinned recipes; disposable probe graduation + retirement; old incremental suite reframing

### Explicitly deferred
- Tolerance-mode combiners (variance/stddev, true-float payloads) — needs its own comparison-soundness design (design §14)
- Spark backend variant of the harness — gated on Spark-parity infrastructure (design §14)
- Sub-day granularity recipes — propagation layer is day-ordinal today
- `materialized_view` shapes beyond the existing hard-error refusal assertion — invariant discharged by engine IVM, not the smelt oracle
- Fixing any production bug the harness discovers — pinned + registered, then fixed via its own red-green change

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1 — Recipe substrate + rendering | done | 82e3f620 | 2026-07-12 |
| 2 — Verdict protocol (admitted/refused) | done | 2661883d | 2026-07-12 |
| 3 — S-tracked oracle + standing gate (append-only, partition grain) | done | b629c4c0 | 2026-07-12 |
| 4 — Mutable sources + settled-point oracle + sensitivity probe | done | | 2026-07-12 |
| 5 — Keyed grain + ledger probes + carve-outs | pending | | |
| 6 — Schedule enrichment | pending | | |
| 7 — Plan-claim probes | pending | | |
| 8 — Simulated change feed | pending | | |
| 9 — Definition-change steps | pending | | |
| 10 — Generated DAGs | pending | | |
| 11 — Graduation & consolidation | pending | | |

---

### Phase 1: Recipe substrate + rendering

**Goal.** `ModelRecipe` as a typed proptest value for the partition-grain append-only pool (pass-through · filter · additive agg · idempotent agg · decomposed agg (`AVG`) · holistic agg), rendered to a stageable project and an oracle query. No execution yet.

**Pre-conditions.** None (first phase).

**TDD tests to write first.**
- `crates/smelt-maintenance-testkit/src/recipe.rs::rendered_recipe_stages_cleanly` — proptest: every generated recipe renders to a staged project whose `file_diagnostics` contain **no parse/type/config errors** (maintenance-family diagnostics permitted); a dirty render is a generator bug, failed loudly.
- `crates/smelt-maintenance-testkit/src/recipe.rs::oracle_sql_is_model_body_with_sources_swapped` — the rendered oracle query equals the model body with each `smelt.sources.<x>` replaced by its physical table name (`main.sources_<x>`), nothing else changed.
- `crates/smelt-maintenance-testkit/src/recipe.rs::payloads_are_integer_valued_and_bounded` — data-generation discipline (design §5): generated payload literals are integer-valued, |v| ≤ the documented bound.
- `crates/smelt-maintenance-testkit/src/recipe.rs::reachability_sample_inhabits_every_pool_construct` — deterministic `TestRunner` sample of N=200 recipes inhabits every `BodyConstruct` variant in the pool and both clocked-source key shapes (pattern: `type_property_tests.rs::reachability`).
- `crates/smelt-maintenance-testkit/src/recipe.rs::recipe_names_its_matrix_cells` — each pool construct maps to ≥1 coverage-matrix `(construct × source-property)` cell id.

**Implementation shape.** New testkit modules: `recipe.rs` (`ModelRecipe`, `SourceRecipe`, `BodyConstruct`, `GrainDecl`, `arb_recipe(RecipePool)`), `render.rs` (`render_project(&ModelRecipe) -> StagedFiles`, `render_oracle_sql(&ModelRecipe) -> String`, `stage(&ModelRecipe, &TempDir) -> LinkCProject`). Rendering follows `model_shapes.rs` frontmatter conventions (no `WHERE start/end`; `smelt.sources.*` refs; source YAML per `sources.md`). Diagnostics self-check reuses a throwaway `smelt_db::Database` (pattern already in `link_c_harness.rs::build_db_and_graph`).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-maintenance-testkit/src/{recipe,render}.rs` — new
- `crates/smelt-maintenance-testkit/src/lib.rs` — module wiring
- `crates/smelt-maintenance-testkit/Cargo.toml` — `proptest` is already a dependency; add none beyond what compiles

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Recipes are valid-by-construction (typed columns, resolvable refs) and shrink structurally
- [ ] Rendering is the single path for model SQL *and* oracle SQL (design §4 "renders once, serves three")
- [ ] No execution-path code (that is Phase 3 scope)

**Commit.** `test(conformance): ModelRecipe substrate + rendering for the append-only partition pool`

---

### Phase 2: Verdict protocol (admitted/refused)

**Goal.** Every staged recipe is classified through the *real* derivation; refusals must carry a named diagnostic (fail-loud check); admissions expose the derived plan to later phases. Adversarial leaf pool added.

**Pre-conditions.** Phase 1.

**TDD tests to write first.**
- `crates/smelt-maintenance-testkit/src/verdict.rs::additive_agg_append_only_admits_delete_insert` — an additive-aggregate recipe over a clocked append-only source derives a `Trigger::NewData` cell with `Technique::DeleteInsert`.
- `crates/smelt-maintenance-testkit/src/verdict.rs::adversarial_leaves_refuse_or_collapse_conservatively` — proptest over the adversarial pool (opaque function call in the event-time slot, `INTERSECT` body, row-nondeterministic function in a skeleton position, symbolic interval in a bound position): each yields either a named refusal diagnostic or a plan whose every admitted cell is full-input recompute — never a targeted technique (spec §Known Divergences `INTERSECT`/`EXCEPT` entry; `model_properties.md` fail-closed constraint).
- `crates/smelt-maintenance-testkit/src/verdict.rs::refusal_without_named_diagnostic_fails_the_case` — the protocol function returns an error (test failure) when a recipe is refused but no `Maintenance*`/admission diagnostic is present.
- `crates/smelt-maintenance-testkit/src/verdict.rs::over_refusal_ledger_records_cell_ids` — refused cases append `(matrix_cell, refusal_kind)` to the in-run ledger; the ledger is reportable (counted, non-gating).

**Implementation shape.** `verdict.rs`: `classify(&LinkCProject, &ModelRecipe) -> Verdict` where `Verdict = Admitted(MaintenancePlan) | Refused(Vec<NamedDiagnostic>)`; consumes `smelt_db::queries::maintenance::derive_model_maintenance_plan` + `file_diagnostics` exactly as production does (maintenance-plan purity: consume, never re-derive). `OverRefusalLedger` is an in-memory tally rendered into the reachability report.

**Critical files.**
- `crates/smelt-maintenance-testkit/src/verdict.rs` — new
- `crates/smelt-maintenance-testkit/src/recipe.rs` — adversarial `BodyConstruct` variants

**Review checklist:**
- [ ] TDD tests exist and assert what's specified
- [ ] Verdicts come from the production derivation entry point, not a harness re-derivation
- [ ] Adversarial pool covers the four named leaf kinds
- [ ] Refusal-without-diagnostic is a hard failure (fail-loud discipline, `architecture.md`)

**Commit.** `test(conformance): admitted/refused verdict protocol + adversarial leaf pool`

---

### Phase 3: S-tracked oracle + standing gate (append-only, partition grain)

**Goal.** Schema-generic data/schedule generation, the S-tracker, the S-restricted per-step oracle, and the standing `maintenance_conformance` test target running the append-only partition pool end-to-end on every `cargo test`.

**Pre-conditions.** Phases 1–2.

**TDD tests to write first.**
- `crates/smelt-maintenance-testkit/src/s_tracker.rs::s_matches_hand_computed_set_on_fixed_schedule` — for a hand-written 3-run schedule with one late append, `S_k` per step equals the hand-computed row multiset.
- `crates/smelt-maintenance-testkit/src/s_tracker.rs::late_row_is_outside_s_until_its_window_reruns` — the spec's horizon semantics fall out of S-tracking with no special-casing.
- `crates/smelt-cli/tests/maintenance_conformance/harness_self_check.rs::oracle_flags_a_seeded_divergence` — after a green run, directly corrupt one output row via a raw connection and assert the oracle reports inequality (the harness-is-not-lying pattern from `nullability_property_tests.rs`).
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs::append_only_partition_pool_upholds_equivalence` — the standing proptest gate: deterministic seed, small N (default 12 cases; `SMELT_CONFORMANCE_CASES` env override), each case = generate recipe → classify → if admitted, generate schedule → drive real `execute_project` per step → assert S-restricted multiset equivalence after **every** run step.
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs::admission_rate_stays_above_floor` — generator health: over the deterministic sample, ≥40% of non-adversarial recipes admit at least one cell.

**Implementation shape.** `schedule_gen.rs`: schema-generic `arb_schedule_for(&ModelRecipe)` replacing the `events(d,id,val)` hard-coding (existing `run_schedule.rs` stays untouched until Phase 11 retirement; new code lives beside it); rows generated per `SourceRecipe` schema with integer-valued payloads and per-source-window placement. `s_tracker.rs`: records `(window, per-source snapshot)` per run; `materialize_s(conn, k)` builds `oracle_<src>` temp tables; oracle = `multiset_equal(conn, "SELECT * FROM <output>", render_oracle_sql over oracle_ tables)`. New test target `crates/smelt-cli/tests/maintenance_conformance/main.rs` (mods: `gate`, `harness_self_check`). Case budget note: each case ≈ 1–3 s (staged project + several runs); 12 cases keeps the target on par with today's `property_discovery`.

**Critical files.**
- `crates/smelt-maintenance-testkit/src/{schedule_gen,s_tracker}.rs` — new
- `crates/smelt-cli/tests/maintenance_conformance/{main,gate,harness_self_check}.rs` — new test target
- `crates/smelt-cli/Cargo.toml` — register the test target (testkit is already a dev-dependency)

**Review checklist:**
- [ ] TDD tests exist and assert what's specified
- [ ] Equivalence asserted after every run step, both directions (`EXCEPT ALL`)
- [ ] Oracle evaluates on a raw `duckdb::Connection`, independent of the run pipeline
- [ ] Gate is deterministic-seeded; env knob scales depth; shrunk failures are pinnable
- [ ] No mutable-source machinery (Phase 4 scope)

**Commit.** `test(conformance): S-tracked per-step oracle + standing maintenance_conformance gate`

---

### Phase 4: Mutable sources + settled-point oracle + sensitivity probe

**Goal.** `mutable_snapshot` sources enter the pool: settled-point oracle mode, multi-source interleave (fact append + dimension mutation), the column-scoped-merge path under generation, and the mutation-sensitivity probe.

**Pre-conditions.** Phase 3.

**TDD tests to write first.**
- `crates/smelt-maintenance-testkit/src/s_tracker.rs::outstanding_mutation_flips_to_settled_point_mode` — mode selection: an in-place update on a mutable source defers full assertion to the next catch-up run covering the affected region.
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs::mutable_pool_settles_to_full_refresh` — fact+mutable-dimension recipes (the `daily_events_enriched` shape, generated): equality at every settled point; expected-staleness in between is recorded, never fatal.
- `crates/smelt-cli/tests/maintenance_conformance/probes.rs::dimension_mutation_touches_only_sensitive_groups` — for an admitted column-scoped-merge cell, mutating only the dimension leaves columns in groups not sensitive to it unchanged (design §7 row 3).
- `crates/smelt-maintenance-testkit/src/schedule_gen.rs::check_profile_verifies_mutable_schedules` — the `check_profile` self-check extends to multi-source interleave steps (declared posture matches actual steps).

**Implementation shape.** `SourceRecipe` gains `mutable_snapshot` posture + `allow_full_scan` rendering where the recipe knows the construct needs it (unclocked dimension); schedule generator emits per-source step streams; `oracle_modes.rs` owns the settled/S-restricted selection per design §6 "mixed models". Sensitivity probe compares pre/post column projections over non-rewritten regions.

**Critical files.**
- `crates/smelt-maintenance-testkit/src/{recipe,schedule_gen,s_tracker,oracle_modes}.rs`
- `crates/smelt-cli/tests/maintenance_conformance/{gate,probes}.rs`

**Review checklist:**
- [ ] TDD tests exist and assert what's specified
- [ ] Settled-point bookkeeping owned by the S-tracker, not per-test logic
- [ ] Sensitivity probe skips (counted) on cells where it structurally can't apply
- [ ] Postures self-checked, never trusted

**Commit.** `test(conformance): mutable sources, settled-point oracle mode, sensitivity probe`

---

### Phase 5: Keyed grain + ledger probes + carve-outs

**Goal.** `grain: key` recipes (keyed fold MERGE through `run_windowed_keyed_maintenance`), the two documented oracle carve-outs, never-fold-twice enforcement, and persisted-reconciliation-store inspection.

**Pre-conditions.** Phase 4.

**TDD tests to write first.**
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs::keyed_pool_upholds_end_state_equivalence` — keyed recipes (additive + idempotent combiner families, key re-touch across windows) equal the oracle's end state at settled points.
- `crates/smelt-cli/tests/maintenance_conformance/probes.rs::redelivered_window_refuses_for_additive_keyed` — re-running a folded window refuses (`KeyedReprocessedWindow`) before the action re-runs (spec §"The reconciliation ledger").
- `crates/smelt-cli/tests/maintenance_conformance/probes.rs::persisted_reconciliation_store_reflects_recompute_reset` — after two `execute_project` runs of a partition-grain recipe, `.smelt/reconciliation.json` contains recompute-reset entries for exactly the recomputed regions (closes design §2 gap 6 — zero integration coverage today).
- `crates/smelt-maintenance-testkit/src/recipe.rs::ordering_keys_are_unique_by_construction` — generator discipline for order-monotone combiners (ties carve-out cannot fire spuriously).
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs::retained_departed_keys_adjusts_the_oracle` — snapshot-reconcile schedules generating deletes compare against oracle rows ∪ retained departed keys (`keyed_models.md` §"End-state equivalence").

**Implementation shape.** Keyed rendering per `model_shapes.rs`'s keyed conventions; carve-out adjustments live in `oracle_modes.rs` keyed variant; ledger inspection deserializes via `smelt-state` types (add dev-dependency to the test target if not already transitively present).

**Critical files.**
- `crates/smelt-maintenance-testkit/src/{recipe,render,oracle_modes}.rs`
- `crates/smelt-cli/tests/maintenance_conformance/{gate,probes}.rs`
- `crates/smelt-cli/Cargo.toml` — `smelt-state` dev-dependency if needed

**Review checklist:**
- [ ] TDD tests exist and assert what's specified
- [ ] Carve-outs implemented as the documented adjustments, not blanket tolerances
- [ ] Ledger probe reads persisted state (the file), not in-memory structures
- [ ] Additive vs idempotent grading both exercised

**Commit.** `test(conformance): keyed-grain pool, ledger probes incl. persisted store, oracle carve-outs`

---

### Phase 6: Schedule enrichment

**Goal.** The remaining adversarial step kinds: re-delivery, explicit backfill, `full_refresh: true` interleave, window-order permutation, and boundary-value data placement.

**Pre-conditions.** Phase 5.

**TDD tests to write first.**
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs::redelivery_of_processed_window_is_idempotent` — partition-grain re-run of the same window never double-counts (DELETE+INSERT full replace).
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs::full_refresh_interleave_resets_state_correctly` — a mid-schedule `full_refresh: true` run resets coverage + ledger such that subsequent incremental runs still uphold equivalence.
- `crates/smelt-cli/tests/maintenance_conformance/probes.rs::window_order_permutations_converge` — two valid orderings of the same window set (same seed, same data) converge to identical final states (order/set-determinacy corollary).
- `crates/smelt-maintenance-testkit/src/schedule_gen.rs::boundary_placement_targets_derived_clamp_edges` — generated rows land just-inside / at / just-outside the admitted plan's scan reach (unit: placement reads the plan's `ScanClamp`).
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs::boundary_rows_within_reach_are_reflected` — a just-inside-reach row appears in the maintained output after its triggering run (an under-derived clamp diverges here).

**Implementation shape.** New `ScheduleStep` kinds in `schedule_gen.rs` (`RerunWindow`, `FullRefreshRun`, `BackfillRegion`); permutation probe generates one schedule, derives a valid permutation, runs both in separate staged projects. Boundary placement is plan-aware: `classify` (Phase 2) runs before data generation.

**Critical files.**
- `crates/smelt-maintenance-testkit/src/{schedule_gen,s_tracker}.rs`
- `crates/smelt-cli/tests/maintenance_conformance/{gate,probes}.rs`

**Review checklist:**
- [ ] TDD tests exist and assert what's specified
- [ ] `check_profile` self-check extended to every new step kind
- [ ] Permutation probe compares full final states, not summaries
- [ ] Boundary placement driven by the derived plan, never a hand-coded margin

**Commit.** `test(conformance): re-delivery, full-refresh interleave, order permutation, boundary placement`

---

### Phase 7: Plan-claim probes

**Goal.** The remaining design-§7 probes: clamp/compiled-SQL consistency, write-window containment, and technique-interchangeability pins. Probe skip-accounting.

**Pre-conditions.** Phase 6 (boundary placement supplies clamp-edge data).

**TDD tests to write first.**
- `crates/smelt-cli/tests/maintenance_conformance/probes.rs::compiled_sql_filter_matches_derived_clamp` — the filter in `SqlCapturingReporter`'s captured SQL matches the admitted cell's `ScanClamp` (plan-vs-execution consistency).
- `crates/smelt-cli/tests/maintenance_conformance/probes.rs::rows_outside_write_window_are_byte_unchanged` — snapshot the output complement region before a run; byte-equal after (spec §Constraints "Write window = output window").
- `crates/smelt-cli/tests/maintenance_conformance/probes.rs::technique_pins_agree_at_fixed_s` — for a cell admitting both families, the same recipe+schedule run under `maintenance.cells[].technique: fold` and `: recompute` yields identical final states (state-preserving modulo ledger for additive; spec §"Per-cell admission" interchangeability).
- `crates/smelt-cli/tests/maintenance_conformance/probes.rs::probe_skips_are_counted_never_silent` — every probe that structurally can't apply to a case increments a per-probe skip counter surfaced in the reachability report; a probe with 100% skips fails the report.

**Implementation shape.** `probes/` module in the testkit: each probe is `fn(&CaseContext) -> ProbeOutcome { Checked(Result), Skipped(reason) }`; the gate folds outcomes. Pin rendering reuses Phase-1 frontmatter knobs.

**Critical files.**
- `crates/smelt-maintenance-testkit/src/probes.rs` — new
- `crates/smelt-cli/tests/maintenance_conformance/{gate,probes}.rs`

**Review checklist:**
- [ ] TDD tests exist and assert what's specified
- [ ] Interchangeability probe uses identical seeds/data across the two pinned runs
- [ ] A pin naming an unadmitted cell still refuses (never silently falls back) — asserted, not assumed
- [ ] Skip accounting wired into the reachability report

**Commit.** `test(conformance): plan-claim probes — clamp consistency, write-window, technique pins`

---

### Phase 8: Simulated change feed

**Goal.** The `SimulatedChangeFeed` step family (design §5): feed-declared sources get base-table mutation + `(op, key, payload, seq)` feed-table bookkeeping; assert today's contract (recompute-only admission; equivalence via recompute).

**Pre-conditions.** Phase 4 (mutable machinery).

**TDD tests to write first.**
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs::change_feed_source_admits_recompute_only` — a `change_feed`-declared source's admitted cells are all full-input re-derivation, never a fold (spec §Known Divergences: no live fold machinery consumes a feed's delta shape).
- `crates/smelt-maintenance-testkit/src/feed.rs::feed_table_records_every_mutation_step` — each mutation step against a feed-declared source appends exactly one feed row; base table and feed replay agree (feed self-check).
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs::feed_declared_source_upholds_equivalence_via_recompute` — mutation schedules over feed-declared sources settle to full-refresh equality.
- `crates/smelt-maintenance-testkit/src/feed.rs::retraction_steps_are_gated_to_feed_sources` — retraction/tombstone step kinds refuse to generate for non-feed postures (self-check).

**Implementation shape.** `feed.rs`: staged feed table `main.feed_<src>(op, key…, payload…, seq)`; driver applies op to base table and appends feed row atomically per step. Nothing in production consumes the feed — this phase asserts the refusal surface and readies generative coverage for future feed-consuming techniques (recorded as such in module docs).

**Critical files.**
- `crates/smelt-maintenance-testkit/src/{feed,recipe,schedule_gen}.rs`
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs`

**Review checklist:**
- [ ] TDD tests exist and assert what's specified
- [ ] Feed bookkeeping self-checked (base-vs-replay agreement)
- [ ] No production code touched (the feed is harness-side only)

**Commit.** `test(conformance): simulated change-feed step family + recompute-only admission assertions`

---

### Phase 9: Definition-change steps

**Goal.** A `RewriteModel` schedule step: the model file changes between runs (column add), asserting today's contract — model-hash change resets interval coverage; the next run recovers equivalence.

**Pre-conditions.** Phase 3 (the driver re-discovers models per run — already the harness's behaviour).

**TDD tests to write first.**
- `crates/smelt-maintenance-testkit/src/schedule_gen.rs::rewrite_model_step_changes_the_hash` — the step rewrites `models/<name>.sql` with an added payload column; `compute_model_hash` differs.
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs::column_add_between_runs_recovers_equivalence` — schedule: runs → `RewriteModel` (add integer payload column) → catch-up runs; final state equals the oracle of the *new* body over full S (interval-store hash invalidation forces the recompute today; the assertion is today's behaviour, not the unbuilt `PureBackfill` classification).
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs::skeleton_position_add_is_refused_or_recomputed_never_corrupted` — adding a column in a grouping position mid-schedule never yields a silently-wrong maintained state: either a named refusal or a full recompute whose result equals the oracle.

**Implementation shape.** `RewriteModel { new_body: RenderedModel }` step; recipes carry an optional `evolution: Vec<ModelEdit>` the generator draws from (payload add; grouping-position add for the refusal case). Oracle SQL re-renders after the rewrite.

**Critical files.**
- `crates/smelt-maintenance-testkit/src/{recipe,render,schedule_gen,s_tracker}.rs`
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs`

**Review checklist:**
- [ ] TDD tests exist and assert what's specified
- [ ] Assertions target today's contract (hash-invalidation recompute), with the unbuilt definition-change classification named in a doc comment, not silently anticipated
- [ ] Oracle re-renders post-rewrite (never compares old body against new output)

**Commit.** `test(conformance): definition-change schedule steps (column add between runs)`

---

### Phase 10: Generated DAGs

**Goal.** 2–3 node generated graphs (chain, diamond, fan-out): forward-propagation sufficiency and backward resolution against full refresh, plus the payload-leak fixture family.

**Pre-conditions.** Phases 3–6 (single-model machinery stable).

**TDD tests to write first.**
- `crates/smelt-cli/tests/maintenance_conformance/dags.rs::chain_since_upstream_dirty_set_suffices` — generated two-hop chain; for generated landed deltas, running exactly what `plan_since_upstream` schedules leaves **every** node multiset-equal to full refresh (executes-and-compares, which `since_upstream_propagation.rs` today does not).
- `crates/smelt-cli/tests/maintenance_conformance/dags.rs::diamond_propagation_suffices` — same over a generated diamond (two paths, one confluence).
- `crates/smelt-cli/tests/maintenance_conformance/dags.rs::include_upstreams_resolved_slices_suffice` — staging exactly the backward-resolved slices and building bottom-up yields a target period equal to a build over complete history.
- `crates/smelt-cli/tests/maintenance_conformance/dags.rs::upstream_payload_in_downstream_skeleton_position` — the leak family: a generated upstream payload column consumed in a downstream `GROUP BY`/`JOIN…ON`; the pair either refuses loudly or upholds equivalence — never silently diverges.

**Implementation shape.** `dag.rs`: `DagRecipe { nodes: Vec<ModelRecipe>, edges }` wiring nodes by model ref; rendering stages one project with N models; drives `plan_since_upstream`/`resolve_build_plan` (`crates/smelt-runtime/src/propagation.rs`) through the CLI-equivalent entry points used by `since_upstream.rs`. Adjointness math stays owned by `maintenance_propagation_adjoint.rs` (pure) — out of scope here.

**Critical files.**
- `crates/smelt-maintenance-testkit/src/dag.rs` — new
- `crates/smelt-cli/tests/maintenance_conformance/{main,dags}.rs`

**Review checklist:**
- [ ] TDD tests exist and assert what's specified
- [ ] Every node compared, not just the sink
- [ ] Landed deltas supplied explicitly (the v1 `--source/--landed` contract), never inferred
- [ ] Keyed-grain nodes excluded from generated graphs (graph refuses them by design — assert the refusal in one case)

**Commit.** `test(conformance): generated DAGs — propagation sufficiency + backward resolution`

---

### Phase 11: Graduation & consolidation

**Goal.** One rendering path and one standing gate: catalogue shapes become pinned recipes; disposable probes' hazard schedules are pinned and the probe files retired; the old incremental suite is reframed; divergence-registry governance lands; spec References and `CLAUDE.md` record the gate; soak is wired.

**Pre-conditions.** Phases 1–10.

**TDD tests to write first.**
- `crates/smelt-cli/tests/maintenance_conformance/pinned.rs::pinned_recipes_reproduce_catalogue_coverage` — for each `model_shapes.rs` shape, a named pinned recipe renders a model exercising the same construct × posture cell, and its gate case is green.
- `crates/smelt-cli/tests/maintenance_conformance/pinned.rs::hazard_schedules_are_pinned` — each retired `g_*`/`sc_*` probe's seeded hazard schedule (late-conversion-within-7d, back-dated in-place update, stacked-frame series reach, CTE-bypass DISTINCT, …) exists as a deterministic pinned case, one per retired probe, doc-commented with the probe it replaces.
- `crates/smelt-cli/tests/maintenance_conformance/registry.rs::divergence_registry_staleness_report` — registry entries that never fired in the deterministic sample are reported (never fail), the type-oracle governance pattern.

**Implementation shape.** `pinned.rs` corpus module (recipe values, not SQL strings); delete the retired probe files + `model_shapes.rs` once parity tests are green; update `crates/smelt-cli/tests/incremental/main.rs`'s header to state its layer (backend strategy execution given a filter) explicitly. Soak: a nightly workflow job (label-gated, mirroring `compat.yml` patterns) running the gate at `SMELT_CONFORMANCE_CASES=200`, plus an autonomy-loop sub-plan stub registered per `docs/autonomy_loop.md`. Spec/doc edits (timeless): `docs/specs/maintenance_plan.md` §References → Tests replaces the `property_discovery` description with the `maintenance_conformance` gate; `CLAUDE.md` architectural-invariants list gains the standing gate line under the maintenance-plan purity entry.

**Critical files.**
- `crates/smelt-cli/tests/maintenance_conformance/{pinned,registry}.rs` — new
- `crates/smelt-cli/tests/property_discovery/` — retire disposable files (keep `coverage_matrix_gaps.rs` and anything the conformance gate does not subsume; move `g_10`'s permanent coverage into `pinned.rs`)
- `crates/smelt-maintenance-testkit/src/{model_shapes,run_schedule}.rs` — retire after parity (superseded by `recipe.rs`/`schedule_gen.rs`)
- `crates/smelt-cli/tests/incremental/main.rs` — header reframe only
- `docs/specs/maintenance_plan.md` — §References → Tests update (timeless)
- `CLAUDE.md` — standing-gate line (timeless)
- `.github/workflows/` — nightly soak job
- `.claude/` — autonomy-loop sub-plan registration for soak sessions

**Review checklist:**
- [ ] TDD tests exist and assert what's specified
- [ ] Nothing retired before its pinned replacement is green (parity first, deletion second)
- [ ] `coverage_matrix_is_inhabited`'s `CLAIMED` entries updated to cite pinned recipes where they replace probes
- [ ] Spec/`CLAUDE.md` edits are timeless — no phase vocabulary
- [ ] Old incremental suite reframed, not deleted

**Commit.** `test(conformance): graduate catalogue + hazard probes into pinned recipes; wire soak; retire disposables`

---

## Blocked phases

(Append-only. The autonomy loop records blocked phases here — phase id, dated reason, candidate options — and continues to the next `pending` phase.)

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the design is satisfied at the end:
- `cargo test -p smelt-cli --test maintenance_conformance` — the standing gate, green at default N
- `SMELT_CONFORMANCE_CASES=200 cargo test -p smelt-cli --test maintenance_conformance --quiet 2>&1 | tail -40` — a deep local soak pass, green
- Reachability report shows: every pool construct, technique family, trigger class, and grain inhabited; admission rate above floor; no probe at 100% skip
- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test maintenance_plan_conformance` — coverage-matrix inventory still partitions cleanly with updated `CLAIMED` citations
- `/smelt:validate maintenance_plan` reports no drift (References → Tests matches reality)
