# Plan: Statement-level lowering & position-scoped emission

**Date**: 2026-08-27
**Spec**: [`docs/specs/multi_backend.md`](../specs/multi_backend.md)
**Spec diff**: uncommitted working tree — §"Emission is scoped to call position" and §"Statement-level lowering" (new); §"Exact-median lowering", §"Cross-engine emission audit", §Constraints (updated); `docs/specs/functions.md` emission API; `docs/specs/architecture.md` item 14; `CLAUDE.md` function-registry invariant
**Tracking PR / branch**: `registry-dialect-emission` (PR TBD) — closes the window-position family of #179
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read the spec at `docs/specs/multi_backend.md` — it is the correctness oracle, in particular §"Emission is scoped to call position" and §"Statement-level lowering". Do not re-open settled spec decisions.
2. Confirm you are on branch `registry-dialect-emission`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**
- Real-fixture tests, not just AST units — every phase exercises its feature in `examples/`.
- Red-green TDD: failing test before any implementation.
- Verification gate is `bash .claude/scripts/verify-phase.sh` — do not run the four commands separately.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope.
- Honor architectural invariants from `CLAUDE.md`.
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this plan file only*.

**Standing warning specific to this plan.** Every lowering here is a *silent-wrong-answer* risk, not a crash risk: an admissibility rule that fails open returns a plausible number. Where a phase's test list names a refusal, the refusal is the deliverable — a phase that lowers more shapes than the spec admits has failed, even with a green suite.

**Live-engine facts already measured** (2026-08-27, live BigQuery + local DuckDB — do not re-litigate these from documentation):
- BigQuery: `PERCENTILE_CONT(x,0.5) OVER (PARTITION BY g)` runs; as an aggregate it is refused (`percentile_cont aggregate function is not supported`); with a window `ORDER BY` it is refused (`Window ORDER BY is not allowed for analytic function percentile_cont`); `PERCENTILE_CONT(0.5) WITHIN GROUP (…)` is a syntax error.
- BigQuery: `MAX_BY(...) OVER (PARTITION BY g)` is refused even partition-only; `APPROX_COUNT_DISTINCT(...) OVER (…)` is refused.
- BigQuery: `IS NOT DISTINCT FROM` works; NaN keys group together *and* compare not-distinct, so the join is total; `ANY_VALUE` over a per-group constant is exact; a synthesised CTE appended to an author `WITH` list works and may reference the author's bindings.
- BigQuery: both target lowerings run end-to-end with correct values; the plain equi-join variant keeps 3 of 5 rows.
- DuckDB: `ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING EXCLUDE CURRENT ROW` returns `5, 4, 3` — not whole-partition. `OVER w` with `WINDOW w AS (PARTITION BY g ORDER BY t)` returns `1, 3` — running.
- Spark's `<=>` spelling is the one engine claim **not** yet measured; Phase 4 must verify it against a live Spark Connect server (`scripts/spark-up.sh`) before relying on it.

---

## Context

smelt's emission verdicts are keyed on dialect alone, so a built-in that a backend supports in one call position and refuses in the other has no way to say so: `print_bigquery_median` peeks at a sibling `WINDOW_SPEC` to recover the position the registry should own. The window-position family of #179 is the consequence — 8 ledger rows across DuckDB, Spark and BigQuery where the engine offers a built-in only in the *opposite* position from the one the author wrote, which no expression-local rewrite can fix. This plan implements §"Emission is scoped to call position" and §"Statement-level lowering".

## Scope

### In scope (spec coverage)
- §"Emission is scoped to call position" — the `Position` axis, no-fallback lookup, the window-verdict totality obligation, and whole-partition classification including `EXCLUDE` frames and named-window resolution.
- §"Statement-level lowering" — the four admissibility rules, both lowering shapes, and the running-window refusal.
- §"Cross-engine emission audit" — the fourth probe position, the joint position/lowering migration, and the coverage-table cell.
- §Constraints — the `RestructureId` dispatch, row-multiplicity, and per-rule refusal gates.

### Explicitly deferred
- **Correlated self-join lowering for running windows.** Rejected in design: quadratic cost and subtle `RANGE` peer semantics. Running windows refuse; §"Statement-level lowering" states this.
- **PostgreSQL verdicts.** `dialect_gaps_postgres` is 0 and no PG row in the family exists; adding positions there is unmotivated churn.
- **Closing the non-window rows of #179** (`AGE`, `DATE_PART`, `SPLIT_PART`, the `EXTRACT` family, …). Plain renames, unrelated to position scoping.
- **`FIRST`/`LAST`** — blocked upstream on #175 (lexed as the `NULLS FIRST` keyword, so they never parse as a call).

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | 650c1760 | 2026-08-27 |
| 2     | done     | 63533bac | 2026-08-27 |
| 3     | done     | 863cca6d | 2026-08-28 |
| 4     | done     | c8a64fed | 2026-08-28 |
| 5     | done     | 750ab24c | 2026-08-28 |
| 6     | done     | aecbd75e | 2026-08-28 |
| 7     | done     | c6eec486 | 2026-08-29 |
| 8     | done     |          | 2026-08-29 |

---

### Phase 1: Call-position classifier

**Goal.** A pure function that answers "what position does this call occupy?" from the source CST, resolving named windows, so that no consumer ever derives position for itself.

**Pre-conditions.** None.

**TDD tests to write first.**
- `crates/smelt-types/tests/registry_coverage.rs::position_variants_are_exhaustive` — `Position` has exactly `Any`/`Scalar`/`Aggregate`/`WholePartitionWindow`/`Window`, and `Any` is documented as a lookup wildcard that no classifier ever returns.
- `crates/smelt-dialect/tests/call_position.rs::scalar_call_is_scalar` — a row-wise call in a plain `SELECT`.
- `crates/smelt-dialect/tests/call_position.rs::aggregate_under_group_by_is_aggregate`
- `crates/smelt-dialect/tests/call_position.rs::partition_only_window_is_whole_partition` — `OVER (PARTITION BY g)`.
- `crates/smelt-dialect/tests/call_position.rs::explicit_unbounded_frame_is_whole_partition` — `ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING`.
- `crates/smelt-dialect/tests/call_position.rs::order_by_without_frame_is_running` — the SQL default frame is `RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW`.
- `crates/smelt-dialect/tests/call_position.rs::exclude_clause_defeats_whole_partition` — `UNBOUNDED … UNBOUNDED EXCLUDE CURRENT ROW` classifies as `Window`. Measured: DuckDB returns `5, 4, 3` for this frame.
- `crates/smelt-dialect/tests/call_position.rs::named_window_is_resolved_before_classifying` — `OVER w` with `WINDOW w AS (PARTITION BY g ORDER BY t)` classifies as `Window`, not `WholePartitionWindow`. Measured: DuckDB returns `1, 3`.
- `crates/smelt-dialect/tests/call_position.rs::unresolvable_named_window_is_running` — an unresolved reference, and an inheriting `OVER (w ORDER BY t)` whose base is unresolved, both classify as `Window`. Refusing is the safe direction.
- `crates/smelt-dialect/tests/call_position.rs::classifies_every_call_in_example_model` — parses a real model under `examples/` and asserts a position for every `FUNCTION_CALL`, so the classifier is total over real source.

**Implementation shape.** `Position` (with `Any`) lands in `crates/smelt-types/src/signatures.rs` beside `Emission`. `crates/smelt-dialect/src/position.rs` exposes `pub fn classify(node: &SyntaxNode, root: &SyntaxNode) -> Position`; `root` is what makes `WINDOW` clause resolution possible. Whole-partition detection is a private helper over `WINDOW_SPEC`, checking `FRAME_EXCLUDE` (`syntax_kind.rs:217`) explicitly. No registry or printer change in this phase.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-types/src/signatures.rs` — add `Position`.
- `crates/smelt-dialect/src/position.rs` — new; the classifier.
- `crates/smelt-dialect/src/lib.rs` — re-export.
- `crates/smelt-dialect/tests/call_position.rs` — new.

**Docs touched.**
- `docs/specs/multi_backend.md` — already carries §"Emission is scoped to call position"; confirm the whole-partition wording matches the implemented rule and correct the spec if they diverge.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Spec rules from §"Emission is scoped to call position" are satisfied, `EXCLUDE` and named-window resolution included
- [ ] `classify` never returns `Position::Any`
- [ ] Architectural invariants honored; classifier is pure, no I/O
- [ ] No scope creep into later phases (no registry or printer changes)
- [ ] Spec edits are timeless

**Commit.** `feat(dialect): classify a call's SQL position, resolving named windows (#179)`

---

### Phase 2: Position-scoped registry emission

**Goal.** Make the emission table key on `(DialectId, Position)` with no cross-position fallback, and remove the position-blind lookup so a caller cannot ask the wrong question.

**Pre-conditions.** Phase 1 — `Position` exists and callers can obtain one.

**TDD tests to write first.**
- `crates/smelt-types/tests/registry_coverage.rs::emission_at_prefers_exact_position_over_any`
- `crates/smelt-types/tests/registry_coverage.rs::emission_at_falls_back_to_any_then_native`
- `crates/smelt-types/tests/registry_coverage.rs::window_positions_never_fall_back_to_each_other` — a `WholePartitionWindow` verdict is not returned for a `Window` call, and vice versa. This is the finding that motivated the rule: falling one way emits a running `MAX_BY … OVER` natively to BigQuery, falling the other refuses a whole-partition `MEDIAN` on Spark that the restructure can serve.
- `crates/smelt-types/tests/registry_coverage.rs::window_verdict_totality` — an entry declaring a verdict at one window position and not the other fails, naming the entry and dialect.
- `crates/smelt-db/tests/integration/registry_consistency.rs::no_position_blind_emission_lookup` — `emission_for` is gone; a grep-style assertion that no production call site takes a dialect without a position.

**Implementation shape.** `Signature::emission` becomes `&'static [(DialectId, Position, Emission)]`; `with_emission` takes triples; `emission_for` is replaced by `emission_at(dialect, position)`. All 24 existing `with_emission` sites migrate mechanically to `Position::Any`. `emission_check.rs::unsupported_emissions` and `printer.rs`'s two emission call sites pass `position::classify(...)`. `print_bigquery_median`'s sibling peek is deleted, and `MEDIAN`'s BigQuery entry states its verdicts per position instead.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-types/src/signatures.rs` — table shape, `emission_at`, 24 site migration, `MEDIAN` position verdicts.
- `crates/smelt-dialect/src/emission_check.rs` — pass a classified position.
- `crates/smelt-dialect/src/printer.rs` — pass a classified position; delete the sibling peek in `print_bigquery_median`.
- `crates/smelt-dialect/tests/emission_ownership.rs` — extend to forbid printer-side position derivation.

**Docs touched.**
- `docs/specs/functions.md` — the `emission_at` entry (already drafted in the spec diff); verify it matches the shipped signature.
- `docs/specs/architecture.md` — item 14 position wording (already drafted); verify.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Spec rules from §"Emission is scoped to call position" are satisfied
- [ ] No `emission_for` survives; no caller derives position itself
- [ ] `MEDIAN` on BigQuery behaves exactly as before for both positions (no regression in `median_lowering.rs`)
- [ ] No scope creep into later phases (no `Restructure` variant yet)
- [ ] Spec edits are timeless

**Commit.** `feat(types): key emission verdicts on (dialect, position) with no cross-position fallback (#179)`

---

### Phase 3: Restructure planning & admissibility

**Goal.** A pure planner that, given a query block and a dialect, returns either a restructure plan or the refusals the spec's admissibility rules require — with nothing printed yet.

**Pre-conditions.** Phases 1–2.

**TDD tests to write first.**
- `crates/smelt-dialect/tests/restructure_plan.rs::analytic_only_in_aggregate_position_plans_direction_b`
- `crates/smelt-dialect/tests/restructure_plan.rs::aggregate_only_in_whole_partition_window_plans_direction_a`
- `crates/smelt-dialect/tests/restructure_plan.rs::running_window_is_refused` — `UnsupportedOnBackend` naming built-in, backend, and the whole-partition requirement.
- One refusal test per admissibility rule (§"Statement-level lowering"), each asserting the *refusal*, not a lowering:
  - `::rollup_grouping_is_refused` — and `cube`/`grouping sets` variants.
  - `::occurrence_in_having_is_refused`, `::occurrence_in_order_by_is_refused`, `::occurrence_in_qualify_is_refused`.
  - `::distinct_argument_is_refused`, `::filter_clause_is_refused`.
  - `::unexpanded_wildcard_is_refused`.
- `crates/smelt-dialect/tests/restructure_plan.rs::ordered_set_desc_inverts_the_fraction` — `PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x DESC)` plans `PERCENTILE_CONT(x, 1 - 0.5)`.
- `crates/smelt-dialect/tests/restructure_plan.rs::inexpressible_nulls_modifier_is_refused`
- `crates/smelt-dialect/tests/restructure_plan.rs::where_is_planned_inside_the_bound_source` — the predicate lands in `__smelt_base`, never on the join. Asserting this at plan level is what stops the wrong-answer case where a filtered-out row holds the maximum sort key.
- `crates/smelt-dialect/tests/restructure_plan.rs::several_windows_share_one_bound_source` — different `PARTITION BY` keys yield one base binding and one grouped CTE + join each.
- `crates/smelt-dialect/tests/restructure_plan.rs::non_deterministic_partition_key_expression_is_refused`
- `crates/smelt-dialect/tests/restructure_plan.rs::plans_real_example_model` — a new `examples/` model exercising both directions plans without refusal.

**Implementation shape.** `Emission::Restructure(RestructureId)` with `WindowToCte` and `AnalyticToCte` lands in `signatures.rs`. `crates/smelt-dialect/src/restructure.rs` mirrors `emission_check.rs`: `pub fn plan(root: &SyntaxNode, dialect: SqlDialect) -> Result<RestructurePlan, Vec<UnsupportedEmission>>`. `RestructurePlan` is data — bound-source binding, grouped CTE bindings (name, partition keys, aggregate), and per-call-site replacement references. Admissibility is a private predicate returning the refusal reason, so each rule has one place to live.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-types/src/signatures.rs` — `Emission::Restructure`, `RestructureId`.
- `crates/smelt-dialect/src/restructure.rs` — new; planner + admissibility.
- `crates/smelt-dialect/src/lib.rs` — re-export.
- `crates/smelt-dialect/tests/restructure_plan.rs` — new.
- `examples/` — a fixture model exercising both directions.

**Docs touched.**
- `docs/specs/multi_backend.md` — confirm the four admissibility rules match the implemented predicate; correct the spec if they diverge.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Every admissibility rule in §"Statement-level lowering" has a refusal test that fails open if the rule is deleted
- [ ] Planner is pure; no printing, no I/O
- [ ] No scope creep into later phases (nothing emits SQL yet)
- [ ] Spec edits are timeless

**Commit.** `feat(dialect): plan statement-level restructures with enumerated admissibility (#179)`

---

### Phase 4: Printer emission of restructured statements

**Goal.** Turn a plan into SQL: synthesised CTEs appended to the author's `WITH` list, qualified base references, and a null-safe join spelled per backend.

**Pre-conditions.** Phase 3.

**TDD tests to write first.**
- `crates/smelt-dialect/tests/window_decorrelation.rs::direction_b_matches_snapshot` — BigQuery; the shape measured as running correctly.
- `crates/smelt-dialect/tests/window_decorrelation.rs::direction_a_matches_snapshot` — BigQuery.
- `crates/smelt-dialect/tests/window_decorrelation.rs::duckdb_ordered_set_window_decorrelates` — the DuckDB direction-A case.
- `crates/smelt-dialect/tests/window_decorrelation.rs::synthesised_cte_appends_to_author_with_list` — a model already starting `WITH a AS (…)` stays valid and the synthesised body may reference `a`. Measured working on BigQuery.
- `crates/smelt-dialect/tests/window_decorrelation.rs::null_safe_join_spelling_per_backend` — `IS NOT DISTINCT FROM` on DuckDB/PostgreSQL/BigQuery, `<=>` on Spark, driven by the capability flag and not by a dialect arm.
- `crates/smelt-dialect/tests/window_decorrelation.rs::no_partition_by_uses_cross_join`
- `crates/smelt-dialect/tests/capability_conformance.rs` — extend for the new flag, matching the §Surface capability matrix.
- `crates/smelt-dialect/tests/emission_ownership.rs::every_restructure_id_is_dispatched` — parsed out of `signatures.rs`, not restated.

**Implementation shape.** A `BackendCapabilities` field for the null-safe equality spelling (a small enum, not a `bool`, since it selects a spelling). `PrintContext` gains the plan; `printer.rs` consumes it at statement assembly, reusing the `WITH`-clause path near `printer.rs:745`. Base references in the outer select are qualified to the bound alias.

**Verification note.** Spark's `<=>` is the one unmeasured engine claim. Before this phase's commit, bring up Spark (`bash scripts/spark-up.sh`, `source scripts/spark-env.sh`) and confirm the spelling, or the capability row is a guess.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-dialect/src/dialect.rs` — the null-safe equality capability + per-backend constructors.
- `crates/smelt-dialect/src/printer.rs` — plan consumption, CTE append, alias qualification.
- `crates/smelt-dialect/tests/window_decorrelation.rs` — new, plus snapshots.
- `crates/smelt-dialect/tests/{capability_conformance,emission_ownership}.rs`

**Docs touched.**
- `docs/specs/multi_backend.md` — §Surface capability matrix gains the null-safe equality row (the conformance test asserts matrix ↔ constructors).

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Spec rules from §"Statement-level lowering" mechanics paragraph are satisfied
- [ ] `printer.rs` holds no name-matched spelling, no `SqlDialect` branch, and no position derivation
- [ ] Spark's `<=>` verified against a live server, not assumed
- [ ] No scope creep into later phases (compile path not yet wired)
- [ ] Spec edits are timeless

**Commit.** `feat(dialect): emit restructured statements with a null-safe join (#179)`

---

### Phase 5: Compile-path wiring & the two correctness gates

**Goal.** Make `print_checked_for` plan before printing, and stand up the gates that keep the lowering honest — output-column invariance and row multiplicity.

**Pre-conditions.** Phase 4.

**TDD tests to write first.**
- `crates/smelt-runtime/tests/dialect_seam.rs::running_window_refused_at_compile_time` — the refusal reaches the user as `UnsupportedOnBackend`, not a warehouse error.
- `crates/smelt-runtime/tests/dialect_seam.rs::no_compile_entry_point_prints_without_planning` — mirrors the existing structural assertion that no direct `smelt_dialect::print` call survives.
- `crates/smelt-runtime/tests/projection_dialect_invariance.rs::decorrelated_model_output_columns_are_identical` — a model using both directions compiles to byte-identical `output_columns` and cast-wrap names across DuckDB, Spark and BigQuery. Fixture must include a `SELECT *` model to prove admissibility rule 4 holds.
- `crates/smelt-runtime/tests/restructure_multiplicity.rs::null_partition_key_preserves_row_count` — a NULL-bearing partition key keeps every row; the equi-join variant is asserted to drop them, so the test fails if the null-safe join regresses. Measured on BigQuery: 3 of 5 kept with a plain equi-join.
- `crates/smelt-cli/tests/example_diagnostics.rs` — the new `examples/` fixture stays diagnostic-free.

**Implementation shape.** `print_checked_for` becomes check → `restructure::plan` → print, threading the plan through `PrintContext`. The refusal list merges with `unsupported_emissions` so a user fixing several sites pays one round trip. A dedicated multiplicity test owns the row-count assertion: the audit's value leg cannot, because `ANY_VALUE` is a registered nondeterministic entry probed on the schema leg only (`dialect_audit/overrides.rs:193`).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-runtime/src/compile.rs` — `print_checked_for`.
- `crates/smelt-runtime/tests/{dialect_seam,projection_dialect_invariance,restructure_multiplicity}.rs`

**Docs touched.**
- `docs/specs/multi_backend.md` — §Constraints gate names must match the shipped test paths.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Nothing re-parses dialect-printed SQL; the plan derives from the source CST
- [ ] The multiplicity gate fails if the join spelling regresses to `=`
- [ ] No scope creep into later phases (no registry verdicts flipped yet)
- [ ] Spec edits are timeless

**Commit.** `feat(runtime): plan restructures before printing; gate projection and multiplicity (#179)`

---

### Phase 6: Registry verdicts, fourth probe position, ledger narrowing

**Goal.** Flip the affected entries to their real per-position verdicts, teach the audit the whole-partition position, and narrow the ledger rows this closes.

**Pre-conditions.** Phase 5. Per §"Cross-engine emission audit", the position and the lowering land together — this phase is where the probe position is added, because only now do the pairs pass it.

**TDD tests to write first.**
- `crates/smelt-db/tests/dialect_audit/main.rs::coverage_totality` — extend for the fourth position; an entry with no probe is named, never dropped.
- `crates/smelt-db/tests/dialect_audit/main.rs::gap_count_ratchet` — tightened baselines.
- `crates/smelt-db/tests/dialect_audit/main.rs::doc_sync` — the generated `docs/reference/dialect-coverage.md` renders a per-position verdict set where positions differ, rather than collapsing them.
- `crates/smelt-db/tests/dialect_audit/` DuckDB legs run in-process per-PR and must stay green for `PERCENTILE_CONT`/`PERCENTILE_DISC` at `WholePartitionWindow`.

**Verdicts to set** (each measured, see the execution prompt):
- BigQuery `PERCENTILE_CONT`/`PERCENTILE_DISC`: `Aggregate` → `Restructure(AnalyticToCte)`, `WholePartitionWindow` → `Native`, `Window` → `Unsupported`.
- BigQuery `ARG_MAX`/`ARG_MIN`: `Aggregate` → `Rename("MAX_BY"/"MIN_BY")`, `WholePartitionWindow` → `Restructure(WindowToCte)`, `Window` → `Unsupported`.
- BigQuery `APPROX_COUNT_DISTINCT`: `WholePartitionWindow` → `Restructure(WindowToCte)`, `Window` → `Unsupported`.
- BigQuery `MEDIAN`: `Aggregate` → `Rewrite(BigQueryMedian)`, `WholePartitionWindow` → `Rewrite(BigQueryMedian)`, `Window` → `Unsupported`.
- DuckDB + Spark `PERCENTILE_CONT`/`PERCENTILE_DISC`, Spark `MEDIAN`: `WholePartitionWindow` → `Restructure(WindowToCte)`, `Window` → `Unsupported`.

**Implementation shape.** `probe.rs::Position` is replaced by the `smelt-types` `Position`; `Probe::statement` gains the `OVER (PARTITION BY g)` shape. Affected `ledger.rs` rows narrow from `Position::Window` to the running case with their reason updated; none is deleted, because the engines genuinely have no window form. `.claude/dialect-gaps-baseline.txt` tightens for `duckdb`, `spark` and `bigquery`.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-types/src/signatures.rs` — the verdicts above.
- `crates/smelt-db/tests/dialect_audit/{probe,ledger,main,report}.rs`
- `.claude/dialect-gaps-baseline.txt`, `docs/reference/dialect-coverage.md`

**Docs touched.**
- `docs/specs/multi_backend.md` — §"Exact-median lowering" verdict list must match what ships.
- `docs/reference/dialect-coverage.md` — regenerated, not hand-edited.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Every entry touched declares verdicts at *both* window positions (totality gate from Phase 2)
- [ ] Ledger rows narrowed, not deleted; each retains its `#179` tracking issue
- [ ] Baselines tightened, never raised
- [ ] Spec edits are timeless

**Commit.** `feat(types): per-position verdicts for the window-position family; narrow the ledger (#179)`

---

### Phase 7: User documentation

**Goal.** Tell users which SQL shapes lower, which refuse, and what to do about a refusal.

**Pre-conditions.** Phase 6 — the shipped behaviour is settled.

**TDD tests to write first.**
- `cargo test -p smelt-cli --test example_diagnostics` — the documented examples are real, compiling models.
- A docs example asserting the refusal message text matches what `UnsupportedOnBackend` actually emits, so the guide cannot drift from the diagnostic.

**Implementation shape.** `docs-site/docs/guide/targets.md` gains a section on aggregates whose support differs by call position: which lower transparently, that a whole-partition window is required, and the rewrite an author can apply by hand when their window must be running. `docs-site/docs/reference/diagnostics.md` and `docs/specs/diagnostics.md` describe the widened `UnsupportedOnBackend` cause.

**Critical files (allowed to touch in this phase).**
- `docs-site/docs/guide/targets.md`
- `docs-site/docs/reference/diagnostics.md`
- `docs/specs/diagnostics.md`

**Docs touched.** As above — written as a feature description, no plan vocabulary.

**Review checklist** (material findings only):
- [ ] Documented SQL actually compiles under `example_diagnostics`
- [ ] The refusal message in the guide matches the emitted diagnostic
- [ ] §Surface of `multi_backend.md` and the user docs agree
- [ ] Docs are timeless — no phase headings or labels

**Commit.** `docs: position-dependent backend support and the whole-partition requirement (#179)`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

- **Interim BigQuery `MEDIAN` verdict at `Position::Window`.** Shipped as `Rewrite(BigQueryMedian)` to keep
  `median_lowering.rs` behaviour identical, while §"Exact-median lowering" already states the target of
  `Unsupported`. Phase 6's verdict list flips it. No automated tripwire forces that flip — the plan's own
  Phase 6 checklist is the only guard, so it must not be skipped.
- **`representative_emission` collapse in the audit report.** `crates/smelt-db/tests/dialect_audit/report.rs`
  picks one verdict per `(entry, dialect)` cell for the published coverage table. Verified inert (no gate
  consumes it) and correct only while an entry's positions agree. Phase 6 replaces it with per-position
  rendering; it becomes wrong the moment a verdict differs by position.
- **`PERCENTILE_CONT`/`PERCENTILE_DISC` registry verdicts landed early, in Phase 3.** Phase 3 added the
  `Position::Window → Emission::Unsupported` verdict for these on DuckDB and Spark (no analytic window
  form exists on either backend) as part of building and testing the restructure planner against a real
  registry table rather than a synthetic one. This was reviewed and adjudicated to stay — reverting to a
  test-only registry would leave the planner tested against a fake table, and refusal is the spec-mandated
  safe direction. Two consequences for whoever executes Phases 5 and 6:
  - (i) This verdict is **already live in production**: `emission_check::unsupported_emissions` is wired
    into `crates/smelt-runtime/src/compile.rs` (~line 661), so a running-window `PERCENTILE_CONT`/
    `PERCENTILE_DISC` now refuses at compile time where it previously reached the warehouse unchecked.
    Nothing landed in Phase 3 pins this new compile-time refusal with a test — that is what Phase 5's
    `crates/smelt-runtime/tests/dialect_seam.rs::running_window_refused_at_compile_time` is for. Do not
    treat that test as pinning a hypothetical; the behaviour it's pinning already shipped in Phase 3.
  - (ii) `crates/smelt-db/tests/dialect_audit/main.rs::is_declared_unsupported` is **position-blind**: it
    only checks `Position::Any`, which is why the Phase 3 `Position::Window` `Unsupported` verdicts did not
    disturb the audit ledger or the exact-match ratchet — the helper simply never saw them. Phase 6 adds
    the fourth probe position and **must** fix this helper to be position-aware, or the audit will silently
    exempt verdicts it should be checking.

### Live-engine measurement (recorded during implementation)

Spark's `<=>` was measured against a live Spark Connect server (Spark 4.0.0) on the decorrelation
join shape with a NULL-bearing partition key: `<=>` keeps all 5 rows, a plain `=` keeps 3 of 5 —
reproducing on Spark the row loss previously measured on BigQuery. Spark 4.0.0 also *accepts*
`IS NOT DISTINCT FROM` (scalar and `JOIN ON`); smelt emits `<=>` per the capability matrix.

- **The planner has no independent running-window guard.** `restructure::plan` trusts the registry
  verdict completely: asked to restructure a call at `Position::Window`, it produces a whole-partition
  CTE and silently drops the running semantics. That is correct under registry single ownership — the
  verdict is the single source of truth — but it means `dialect_seam::running_window_refused_at_compile_time`
  is the ONLY tripwire. Verified during review by flipping the DuckDB verdict and watching the compile
  succeed with wrong semantics.
- **The multiplicity gate's blast radius.** `restructure_multiplicity` catches a join-spelling regression
  to `=`. It would NOT catch a regression to `LEFT JOIN`, nor a CTE grouped on the wrong key — both can
  still yield 5 rows. Widening it needs a value assertion, which the audit's value leg cannot own because
  `ANY_VALUE` is registered nondeterministic and probed on the schema leg only.

- **The gap ratchet cannot move for this family, by design.** `dialect_gaps_*` counts ledger *rows*,
  and every affected entry still genuinely refuses the *running* case — a distinct position from
  whole-partition. Rows narrow rather than disappear, so `.claude/dialect-gaps-baseline.txt` correctly
  stays at duckdb 12 / spark 27 / bigquery 42. Phase 6's "baseline tightens" wording was loose; the
  closure this plan delivers shows up as narrowed row scope and per-position coverage cells, not as a
  smaller count.
- **`is_declared_unsupported`'s position fix is currently inert.** Reverting it to `Position::Any`
  fails no test, because `is_registered` already consults the real position and masks it. It is correct
  defense-in-depth for an `Unsupported`-without-ledger-row entry, which does not yet exist. Per-PR CI
  cannot distinguish "correct" from "redundant" here.

### Phase 8: BigQuery verdict correction (added after a live sweep)

The live BigQuery sweep run after Phase 6 contradicted the whole-partition verdict for
`PERCENTILE_CONT`/`PERCENTILE_DISC`. `Emission::Native` prints verbatim, and smelt spells these as
ordered-set aggregates, so BigQuery received `WITHIN GROUP` and refused it
(`Syntax error: Expected "(" but got keyword GROUP`). The plan's measured fact was about BigQuery's
own two-argument analytic spelling, not about smelt's spelling printed verbatim. Shipped as
`Rewrite(WithinGroupToAnalytic)`, converting the ordered-set form to the analytic form in place —
no CTE, since the window is already there — reusing the `DESC` fraction inversion and the `NULLS`
refusal from the restructure planner. Confirmed by a second live sweep.

**A measurement trap, recorded so it is not repeated.** The same sweep appeared to show
`APPROX_COUNT_DISTINCT` accepted in analytic position, and the verdict was changed on that basis.
It was wrong: BigQuery's **dry run accepts** the analytic form and only **execution** refuses it
(`Analytic function APPROX_COUNT_DISTINCT is not supported`). A dry-run probe cannot see this gap;
only the value leg can. The change was reverted before commit and the entry's ledger row restored to
its value-leg classification. Never "correct" this verdict on dry-run evidence.

A third bug surfaced while fixing the first: `push_trailing_trivia` filtered to direct-child tokens
before reversing, splicing two unrelated trivia gaps together when a node had an intervening clause
child. Fixed to walk in document order.

## Verification

How to confirm the spec is satisfied at the end:
- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-dialect` — classifier, planner, decorrelation snapshots, emission ownership
- `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance --test restructure_multiplicity`
- `cargo test -p smelt-db --test dialect_audit` — DuckDB legs, coverage totality, ratchet, doc-sync
- Spark legs: `bash scripts/spark-up.sh && source scripts/spark-env.sh && cargo test -p smelt-cli --features smelt-cli/spark`
- BigQuery sweep (manual tier, bills): `bash scripts/bigquery-auth.sh && bash scripts/bigquery-dialect-audit.sh`
- `/smelt:validate multi_backend` reports zero drift
