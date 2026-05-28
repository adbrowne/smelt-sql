# Plan: type generator-emitted models and route consumers through them

**Date**: 2026-05-28
**Spec**: [`docs/specs/meta_language.md`](../specs/meta_language.md)
**Spec diff**: §"Multi-model production" rule 7 (the `columns:` clause on emitted-model `ModelRef`) is enforced rather than aspirational; §Known Divergences gains an entry recording the previously-silent consumer-typing gap (then has it removed as each phase closes the corresponding piece).
**Tracking PR / branch**: branch `worktree-unknown_types`.
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/meta_language.md` §"Multi-model production" — specifically rules 4 (the W4 stage) and 7 (emitted-model `smelt.<path>` resolution and `columns:`), and the §Status / §Known Divergences sections. These are the correctness oracle.
2. Confirm you are on branch `worktree-unknown_types`. If not, ask before continuing.
3. Find the next `pending` phase in the Progress table. If all are `done`, run Verification and stop.

**Per-phase loop (`/smelt:implement`):** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**
- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule (update the spec via `/smelt:spec` first).
- A pre-existing failure unrelated to this plan surfaces (e.g. a generator-evaluation cycle issue).
- The fix shape grows beyond `smelt-db`'s `queries/`, `type_inference/`, and `lib.rs` — flag before widening (the parser and core crates should not need to change).

**Conventions every phase:**
- Red-green TDD; typing tests drive the *real* `model_function_type` / `typed_model_schema` Salsa queries, not sub-helpers.
- Real-fixture coverage: `examples/per_cohort_union/` is the existing reproducer; the consumer to verify is `models/all_cohorts_unioned.sql`. Additional minimal fixtures may be added under `examples/` if the existing one is over-constrained.
- Atomic per-phase commit using the `Commit.` line verbatim; push after each.
- Honor `CLAUDE.md` invariants: `smelt-db` pure-function rule; Workspace-Loading-Parity Rule; Project Isolation Rule.
- **Timeless-oracle rule.** Phase vocabulary lives in this plan only. Spec / `docs-site` edits describe the feature as if it has always existed — no `Phase N` vocabulary in body sections. As each gap closes, delete its §Known Divergences entry from `meta_language.md` in the same commit.

---

## Context

`examples/per_cohort_union/models/cohorts.gen.sql` is a generator file whose body evaluates to `List<ModelDef>`, emitting three models: `smelt.cohorts.us_west`, `smelt.cohorts.us_east`, `smelt.cohorts.eu`. The downstream consumer `models/all_cohorts_unioned.sql` is a `UNION ALL` over these three emissions. With the recent VALUES typing fix landed (commit `01fc027f`), the underlying-data model `models/orders.sql` resolves all five columns concretely. But `target/debug/smelt type --project-dir examples/per_cohort_union` shows:

```
all_cohorts_unioned:
  (us_west: {created_at, id, region, revenue, user_id})
  -> {id: UNKNOWN, user_id: UNKNOWN, region: UNKNOWN, revenue: UNKNOWN, created_at: UNKNOWN}
```

The investigation (see `agentId: ac944faafcc403558`) traced the gap to two compounding root causes plus one orthogonal symptom:

1. **No typed schema is computed for emitted models.** `evaluate_generator` (`crates/smelt-db/src/queries/project.rs:1095`) materialises each `ModelDef { … }` literal into an `EmittedModelDef` whose `body: TableExpr` field is reduced to a raw `body_text: String`. The SQL inside is never parsed and typed. No `typed_model_schema_for_emitted` query exists; W3's `emitted_models(db, workspace)` returns survivors carrying only `body_text`. Spec rule 7 calls for `columns: the model's column list, derived from ModelDef.body's synthesised schema` — this is currently unimplemented.

2. **The FROM-clause typing path does not consult the emission registry.** `SalsaRefSchemaProvider::resolved_columns` (`schema.rs:566`) calls `resolve_ref` (`lib.rs:431`) — a workspace-flat lookup that scans `parse_model().name` over `SourceFile`s. Emitted models do not have backing `SourceFile`s, so the lookup returns `None`. The richer `resolve_ref_path` (`lib.rs:495`) *does* consult `emitted_models(db, ws).survivors`, but `process_table_ref_pure`'s `SMELT_PATH_REF` branch never calls it. The result: `smelt.cohorts.us_west` falls through both `seed_columns` and `resolved_columns`, no columns are seeded into `ctx.model_columns`, and the outer `SELECT id, …` projects every column as `Unknown`.

3. **Orthogonal: `model_input_constraints` does not walk set-operation chains.** `schema.rs:1462`-`schema.rs:1493` collects FROM entries from the first `SelectStmt` only and ignores `set_operation_select()` continuations. This is why only `us_west` (the leading branch) appears in the displayed input list — `us_east` and `eu` are silently dropped from the input-constraint vector. Closing this is a small symmetric fix; it would still be a bug after #1 and #2 land (the input position of the second / third branches would type as `Unknown` because the constraints were never recorded).

The structural asymmetry with `smelt.sources.*` (commit `1ed38a1e`) is informative: sources have both a dedicated routing branch in `process_table_ref_pure` *and* a side-channel installer (`add_source_info_to_type_context`) that pre-populates `ctx.source_columns` from a separate registry. Emissions need the equivalent of both: a typed-schema product (step 1) and a routing/installation path (step 2). Step 3 is a separate input-constraint walk.

## Scope

### In scope (spec coverage)
- `meta_language.md` §"Multi-model production" rule 7 — the `columns` clause on emitted-model `ModelRef`s becomes enforced: emitted-model columns are derived from `ModelDef.body`'s synthesised schema and reach consumers' typed schemas through the standard `smelt.<path>` resolution.
- `meta_language.md` §"Multi-model production" rule 4 (W4) — downstream consumers of generator-emitted models resolve typed schemas, not just existence.
- `meta_language.md` §Status / §Known Divergences — the consumer-typing gap is recorded (then removed phase-by-phase as it closes).

### Explicitly out of scope
- Generator-body re-evaluation triggered by an upstream-model schema change. Salsa already invalidates correctly when sources change; the question of incremental invalidation of emitted-model schemas as `models/orders.sql` evolves is automatic via the standard Salsa edges, but not specifically tested here beyond a regression case.
- LSP hover/goto-def for emitted-model column references. The spec catalogues these under separate LSP items; the typing-only fix here will incidentally enable them, but no new LSP behaviour is added in this plan.
- Generator-body bodies that reference each other (`A` emits a model that `B`'s body reads from). The spec already forbids `smelt.models.*` inside a generator body (rule §"Generator body restrictions", `GeneratorBodyForbidsModelReflection`), so this case is structurally impossible.
- `ColumnRef` reflection over emitted models (e.g. `smelt.columns_of(smelt.cohorts.us_west)`). Mentioned in the spec but not exercised by the reproducer; defer until concrete pressure.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | 3100af45 | 2026-05-28 |
| 2     | done     | 37bc3845 | 2026-05-28 |
| 3     | done     | c3827ce7 | 2026-05-28 |

---

### Phase 1: Compute typed schemas for emitted models

**Goal.** Each `EmittedModelDef` in `emitted_models(db, workspace).survivors` carries (or is reachable through a Salsa-cached query keyed on it) a `ModelSchema` synthesised from `ModelDef.body`. The schema reflects the SQL's actual column shape — column names from the SELECT list, types from inference against the generator file's project scope. Body SQL referencing hand-authored models / sources / loaders resolves correctly via the standard `SalsaRefSchemaProvider`. The synthesised schema is *not* yet consumed by consumers (Phase 2 wires that); Phase 1 only produces the product and the test gate that proves the product is correct.

**Pre-conditions.** None.

**TDD tests to write first.**

1. `crates/smelt-db/tests/emitted_model_typed_schema.rs` (new integration test). Cases:
   - Single emission, fully concrete body: a generator file emits `ModelDef { name: 'cohort_a', body: SELECT 1 AS id, CAST('us' AS TEXT) AS region }`. Asserting on whatever the new query / accessor returns: `{id: Integer, region: Text}` (use the existing compatibility helper for Text/Varchar if one exists).
   - Body that reads an upstream typed model: generator emits `ModelDef { name: 'cohort_a', body: SELECT id, region FROM smelt.orders WHERE region = 'x' }`, with `models/orders.sql` declaring `id: Integer, region: Text`. Asserting the emission's schema is `{id: Integer, region: Text}` — i.e. propagation through the consumer's `SalsaRefSchemaProvider` is exercised.
   - Multiple emissions in one generator file: asserting each emission's schema is computed independently (no cross-contamination).
   - Body with a column inference gap (e.g. `SELECT id, some_unbound_col FROM …`): asserting the schema produced is partially typed and (per the no-silent-Unknown invariant) the W2 diagnostics include the appropriate identifier-missing code at the emission's location.
2. `crates/smelt-cli/tests/example_diagnostics.rs` — `examples/per_cohort_union/` regression assertion at the emitted-model level: each of `cohorts.us_west`, `cohorts.us_east`, `cohorts.eu` has five typed columns (use the not-Unknown form like the per_cohort_union assertion landed in commit `01fc027f`).
3. Regression: `example_diagnostics` (currently 79) and `example_workspaces` (21) stay green.

**Implementation shape.**
- Extend `EmittedModelDef` (in `crates/smelt-db/src/queries/project.rs`) to carry a `typed_schema: ModelSchema` (or `Arc<ModelSchema>` for cheap cloning). Computed inside `evaluate_generator` after the `ModelDef.body` text is extracted: parse the body SQL as a `SelectStmt` (the body type is `TableExpr` — investigate the existing `body_text` extraction path to confirm it yields a parseable fragment), build a `TypeContext` against the generator file's project scope, and run `infer_select_output_schema` (or the equivalent reachable from the FROM-typing path).
- *Alternatively* — if attaching the schema to `EmittedModelDef` widens cache invalidation too aggressively — introduce a separate Salsa query `emitted_model_typed_schema(db, workspace, generator_file: SourceFile, name: String) -> Arc<ModelSchema>` that recomputes per emission. The implementer must investigate caching trade-offs and pick the shape that minimises spurious invalidation; report the decision. Either shape satisfies Phase 1 as long as consumers can read the schema in Phase 2.
- Keep the analysis pure: parsing + typing of the body SQL must not call back into Salsa from inside the pure helper. The Salsa query / W2 stage does the orchestration.
- Body parsing: investigate whether the existing `evaluate_body_emissions` / `body_text` extraction yields a string that round-trips through `parse_file` cleanly. If a re-parse path doesn't exist, add one — but report before doing so (it may be cheaper to keep the body's CST from W2 rather than re-parse).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/src/queries/project.rs` — W2 extension and / or new Salsa query.
- `crates/smelt-db/src/type_inference/multi_model.rs` — body-schema inference helper (existing helper may already cover much of this; investigate).
- `crates/smelt-db/src/queries/schema.rs` — only if the typed-schema product needs a small accessor surface (e.g. exposing `ModelSchema` cleanly to Phase 2's consumers).
- `crates/smelt-db/tests/emitted_model_typed_schema.rs` — new integration test.
- `crates/smelt-cli/tests/example_diagnostics.rs` — typed-schema assertion for emitted cohorts.

**Docs touched (timeless phrasing — no plan/phase vocabulary in body).**
- `docs/specs/meta_language.md` — §"Multi-model production" rule 7's `columns` clause becomes describing the *implemented* behaviour, not the goal. Add a corresponding §Known Divergences entry (in *behavioural* terms) for any remaining gap (Phase 2 will close consumer routing; Phase 1 alone closes only the production side).

**Review checklist (material findings only):**
- [ ] TDD tests drive the real `evaluate_generator` / `emitted_models` queries and assert on the typed schema.
- [ ] Body SQL referencing hand-authored upstream models resolves through `SalsaRefSchemaProvider` correctly.
- [ ] Multiple emissions in one file are independent (no shared mutable state).
- [ ] Schema-inference gaps inside a body produce diagnostics at the W2 layer — no silent `Unknown`.
- [ ] `smelt-db` pure-function rule preserved (analysis logic stays pure).
- [ ] `example_diagnostics` + `example_workspaces` stay green.
- [ ] Caching shape chosen does not cause spurious cross-emission invalidation.
- [ ] Spec edits are timeless (no `Phase X`).

**Commit.** `feat(types): synthesise typed schemas for generator-emitted models`

---

### Phase 2: Route `smelt.<emitted-path>` consumers through the emission registry

**Goal.** A consumer's `FROM smelt.<dir>.<base>.<name>` reference where the path identifies a generator-emitted model resolves to the emission's typed schema (from Phase 1) and binds the columns into the consumer's `TypeContext`. The consumer's projection of those columns types concretely, not `Unknown`. The existing `smelt.<model>` and `smelt.sources.*` resolution paths are unaffected.

**Pre-conditions.** Phase 1 done.

**TDD tests to write first.**

1. `crates/smelt-db/tests/emitted_model_consumer_typing.rs` (new integration test). Cases:
   - Consumer reads from a single emission: `SELECT id, region FROM smelt.cohorts.us_west` resolves to `{id: Integer, region: Text}` (typed, not `Unknown`).
   - Consumer's `UNION ALL` over multiple emissions: each branch's columns type concretely.
   - Consumer using an alias on an emission: `FROM smelt.cohorts.us_west AS c SELECT c.id, c.region` resolves through the alias to typed columns.
   - Negative regression: a hand-authored model whose leaf name happens to match an emission leaf still resolves to the *hand-authored* model (the spec already names hand-authored as winning per rule `ModelDefHandAuthoredCollision`).
   - Negative regression: `FROM smelt.sources.<path>` continues to route through the sources branch (Phase B's fix at `schema.rs:826` does not regress).
2. `crates/smelt-cli/tests/example_diagnostics.rs` — `examples/per_cohort_union/all_cohorts_unioned.sql` types its five output columns concretely (not `Unknown`). This is the headline closure assertion.
3. Regression: `example_diagnostics` + `example_workspaces` stay green; no existing model that resolves through `smelt.<path>` regresses.

**Implementation shape.**
- Investigate `lib.rs:495`'s `resolve_ref_path` — it already consults `emitted_models(db, ws).survivors`. The question is the API surface: `process_table_ref_pure` and `SalsaRefSchemaProvider::resolved_columns` currently receive a single `model_name: &str` (the leaf), not the full `segments: &[String]`. Extending the resolver to take the full segments is the cleanest fix. Two equivalent shapes:
  - (a) Add a new method `resolved_columns_for_path(segments: &[String]) -> Option<Vec<(String, TypedColumn)>>` on `RefSchemaProvider`; the existing `resolved_columns(name)` becomes the thin compatibility caller (or is removed and all call sites migrate).
  - (b) Change the signature of `resolved_columns` to take the full segments; update the two call sites.
  Pick the shape that minimises churn; report the decision.
- Inside the resolver, after the hand-authored `resolve_ref` lookup returns `None`, consult `emitted_models(db, ws).survivors` via the full path. Return the emission's typed schema (from Phase 1) on match.
- In `process_table_ref_pure` (`schema.rs:807`-`schema.rs:847`), pass the full `segments` to the resolver; on hit, bind columns into `ctx.model_columns` and register the alias the same way the existing hand-authored / seed path does.
- Consider whether emission-routing belongs as a *third early-return branch* (parallel to the `sources` branch) or as a fall-through after `resolve_ref`. Either is defensible; the symmetry argument favours an early-return branch (gives emissions a dedicated routing path that is structurally hard to shadow). Pick one and document why.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/src/queries/schema.rs` — resolver API + `process_table_ref_pure` routing.
- `crates/smelt-db/src/lib.rs` — `resolve_ref_path` reuse / extension as needed.
- `crates/smelt-db/tests/emitted_model_consumer_typing.rs` — new integration test.
- `crates/smelt-cli/tests/example_diagnostics.rs` — typed-schema assertion for `all_cohorts_unioned`.

**Docs touched.**
- `docs/specs/meta_language.md` — remove the Phase 1-added §Known Divergences entry for consumer-side routing.
- `docs-site/docs/reference/language.md` (or the closest user-doc — investigate) — short note that generator-emitted models are typed and addressable identically to hand-authored models, with the standard `smelt.<path>` resolution.

**Review checklist (material findings only):**
- [ ] TDD tests drive the real `model_function_type` / `typed_model_schema` Salsa query.
- [ ] `examples/per_cohort_union/all_cohorts_unioned.sql` types all five output columns concretely.
- [ ] `smelt-db` pure-function rule preserved.
- [ ] Hand-authored collision rule (per `ModelDefHandAuthoredCollision`) still wins for hand-authored models.
- [ ] `smelt.sources.*` routing not regressed (the Phase B fix at `schema.rs:826` continues to fire first).
- [ ] `example_diagnostics` + `example_workspaces` stay green.
- [ ] Resolver API change (if signature changes) is minimal and surfaced in the commit body.
- [ ] Spec + user-doc edits are timeless.

**Commit.** `feat(types): consumers of generator-emitted models receive typed schemas`

---

### Phase 3: `model_input_constraints` walks set-operation chains

**Goal.** `model_input_constraints` aggregates FROM-clause entries across the full set-operation chain (UNION / INTERSECT / EXCEPT and their `ALL` variants), not just the leading SELECT. Each branch's input-position columns receive constraints derived from the resolved schema of the table reference in that branch. The displayed input list for `examples/per_cohort_union/all_cohorts_unioned.sql` includes all three cohort emissions (currently only the first appears).

**Pre-conditions.** Phases 1 and 2 done (so the displayed input columns can be typed, not just discovered).

**TDD tests to write first.**

1. `crates/smelt-db/tests/model_input_constraints_set_ops.rs` (new integration test). Cases:
   - Two-branch UNION ALL: `SELECT … FROM smelt.foo UNION ALL SELECT … FROM smelt.bar` produces input entries for *both* `foo` and `bar`. Their typed columns appear in the input constraints (using Phase 1+2 to ensure types resolve).
   - Three-branch UNION ALL (the reproducer): `examples/per_cohort_union/all_cohorts_unioned.sql` displays three input entries `us_west`, `us_east`, `eu`, each with five typed columns.
   - INTERSECT and EXCEPT variants: same shape, different keyword.
2. Regression: `model_input_constraints` on a single-SELECT model is unchanged.
3. `target/debug/smelt type --project-dir examples/per_cohort_union` shows three input entries with typed columns for `all_cohorts_unioned`.

**Implementation shape.**
- In `crates/smelt-db/src/queries/schema.rs:1462`-`schema.rs:1493`, walk the `set_operation_select()` chain from the root `SelectStmt`. Investigate whether the AST exposes a chain iterator (`SelectStmt::set_operation_chain()`?) or whether the walk is a manual `while let Some(next) = current.set_operation_select() { … current = next; }` loop. Pick the cleanest existing pattern.
- For each branch, collect FROM entries the same way the existing leading-SELECT walk does. Union the entries before constraint construction (deduplicate by path).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/src/queries/schema.rs` — `model_input_constraints` extension.
- `crates/smelt-db/tests/model_input_constraints_set_ops.rs` — new integration test.

**Docs touched.** Spec change is implicit in fixing the input-list shape; no §Surface or §Semantics change is needed in `types.md` or `meta_language.md` (the existing description of `model_input_constraints` does not say "only the leading branch"). No user-doc change required.

**Review checklist (material findings only):**
- [ ] All set-operation branches contribute input entries; deduplication is correct.
- [ ] `examples/per_cohort_union/all_cohorts_unioned.sql` displays three input entries with typed columns.
- [ ] No regression on single-SELECT models or models without UNION.
- [ ] `smelt-db` pure-function rule preserved.

**Commit.** `fix(types): model_input_constraints walks set-operation chains`

---

## Deferred during implementation

(Append-only.)

- Generator-body re-evaluation triggered by upstream-model schema invalidation: covered automatically by Salsa edges in the chosen Phase 1 caching shape; no separate test gate beyond the Phase 1 regression case.
- LSP hover / goto-def for emitted-model column references: spec-deferred to separate LSP plan; the typing-only fix here enables it implicitly.
- Generator-body bodies referencing each other (mutually-emitting generators): structurally forbidden by `GeneratorBodyForbidsModelReflection`.
- `ColumnRef` reflection over emitted models: defer until concrete pressure.

## Verification

- `target/debug/smelt type --project-dir examples/per_cohort_union` shows no `UNKNOWN`: `orders`, `cohorts.us_west|us_east|eu`, and `all_cohorts_unioned` all type concretely; `all_cohorts_unioned`'s input list displays three branches.
- `cargo test -p smelt-cli --test example_diagnostics` — green.
- `cargo test -p smelt-lsp --test example_workspaces` — green.
- `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test` — green.
- `/smelt:validate meta_language` — no drift on the rule-7 `columns:` clause; no stale Known Divergences entry for emission consumer typing.
