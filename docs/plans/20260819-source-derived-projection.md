# Plan: the projection has one owner — the source CST

**Date**: 2026-08-19
**Spec**: [`docs/specs/multi_backend.md`](../specs/multi_backend.md)
**Spec diff**: none yet — the spec edits ride with Phase 1 (§"Output-schema type conformance" gains the projection-derivation rule and the alias-synthesis rule; §"Whole-row MERGE" corrects where `CompiledModel::output_columns` is derived from; §Known Divergences retires the median entry's "general hazard remains recorded here rather than fixed" paragraph). Phase 4 adds a `DiagnosticCode` row to [`docs/specs/diagnostics.md`](../specs/diagnostics.md).
**Tracking PR / branch**: `bigquery-backend-research`
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/multi_backend.md` §"Output-schema type conformance" and §"Whole-row MERGE" — they are the correctness oracle. Do not re-open settled spec decisions.
2. Confirm you are on branch `bigquery-backend-research`, in the worktree `/home/andrew/smelt-sql/.claude/worktrees/bigquery`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- Phase 4's alias-synthesis rule turns out to change output column names in a way the existing example workspaces or the maintenance conformance gate depend on in a way this plan did not anticipate. Report what broke; do not quietly narrow the rule.
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan. One is already known: `cargo test -p smelt-logical --test contract_lattice_spec` fails at `HEAD` for a missing spec heading and predates this branch — do not fix it here.

**Conventions every phase:**
- Real-fixture tests, not just AST units — every phase exercises its feature in `examples/` or against a real backend.
- Red-green TDD: failing test before any implementation.
- Verification gate is `bash .claude/scripts/verify-phase.sh` (one call: fmt + clippy + tests + example_diagnostics, failures-only output) — do not run the four commands separately.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope.
- Honor architectural invariants from `CLAUDE.md`.
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this plan file only*. Edits to `docs/specs/` and `docs-site/docs/` describe the feature as if it has always existed.

---

## Context

`smelt_dialect::print` lowers smelt SQL to target-dialect forms — `MEDIAN` becomes a GoogleSQL `ARRAY_AGG`-indexing expression, `%` becomes `MOD()`, `**` becomes `POWER()`. Several consumers then feed that *printed output* back into `smelt_parser::parse`, asking smelt's parser to read SQL it was never designed to read. Every one of them fails soft, so the damage is silent.

The confirmed sites:

| Site | Flow |
|---|---|
| `smelt-runtime/src/compile.rs:1459` `apply_type_casts` | re-parses printed SQL to recover names and re-run type inference; reached from `:1436`, `:1622`, `:2174` |
| `smelt-runtime/src/compile.rs:438` `output_column_names` | re-parses printed SQL for the output column list; reached from `:1448`, `:1565`, `:1657`, `:2213` |
| `smelt-logical/src/maintenance/emit.rs:1295` `emit_count_preservation_probe_from_body` | re-parses `CompiledModel.sql` to splice out an enrichment join |

Three symptoms trace to this one cause.

**The median bug** (already recorded in `multi_backend.md` §Known Divergences). A BigQuery median arrives at the wrapper as `(CAST(x AS FLOAT64) + CAST(y AS FLOAT64)) / 2`; `FLOAT64` is a spelling smelt's type parser does not recognise, so both operands go unresolved and the wrap emitted `CAST(med_val AS SMALLINT)` — an exact median left the warehouse rounded. The division-promotion half was fixed in `970ef87a`; the spec entry closes with "the general hazard remains recorded here rather than fixed". This plan is that fix.

**The `_colN` leg**, parked as an "orthogonal defect" in three test files (`meta_workspace_e2e.rs:20`, `meta_config_e2e.rs:20`, `example_builds.rs:62`). It is not orthogonal. When a select item has no alias, `apply_type_casts` (`:1516`) invents `_col{i}` and then *references* it from the wrapper — a name the inner query never exposes. It is dialect-independent, and it is why in-model list spread cannot be built end-to-end.

**A dead safety probe.** `emit_count_preservation_probe_from_body` is fed `CompiledModel.sql`, which `apply_type_casts` has already wrapped as `SELECT CAST(..) FROM ( <body> ) _smelt_typed` — so the join it looks for is buried in a derived table and `from_clause.joins()` finds nothing. Verified with a scratch test: the same body returns `Some` unwrapped and `None` wrapped. The function fails closed to a `tracing::warn!` and a widened scan, so it is not a correctness bug — but the count-preservation probe and its delta restriction have never fired in production, on any dialect. Every existing test of that function feeds a hand-written unwrapped body, which is why it looks healthy.

Underneath all three is one DuckDB-shaped assumption. `Expression::infer_name` (`smelt-parser/src/ast.rs:2146`) names an unaliased function call by its **full source text** — `SUM(x)` becomes the column name `SUM(x)`. That works only because it coincides with DuckDB's own default naming. BigQuery names the same column `f0_`. The projection has effectively been derived from one engine's conventions and then asserted as portable.

The fix is a single-ownership move: **a model's projection — its column names and their inferred types — is derived once, from the model's own source CST, before printing.** The printer's job is to render; it is not a source of truth to be read back.

## Scope

### In scope
- `multi_backend.md` §"Output-schema type conformance" — the projection-derivation rule and the alias-synthesis rule.
- `multi_backend.md` §"Whole-row MERGE" — `CompiledModel::output_columns` derives from the source select list, which is what makes its stated "the build path and the editor agree" rationale true.
- `multi_backend.md` §Known Divergences — retire the median entry's unfixed-hazard paragraph.
- `diagnostics.md` — one new `DiagnosticCode` for a user alias claiming the reserved `_smelt_` prefix.
- All four `SqlCompiler` compile entry points.
- `emit_count_preservation_probe_from_body`'s caller.
- A standing gate that the pattern cannot return.

### Explicitly deferred
- **`prepend_ephemeral_ctes` (`compile.rs:1922`).** It does textual `WITH`-detection on printed SQL — category-(c) of the same family — but `WITH` spelling is dialect-invariant, so it is a style problem, not a defect. Fixing it means threading ephemerals through the printer, which is its own piece of work.
- **`cube_split::rewrite` + `smelt-cli/src/executor.rs::execute_plan`.** The inverse problem (raw smelt SQL reaching a backend *without* lowering). Both are dead code today — `execute_plan` has no call sites — so this is a landmine to note, not a bug to fix. Recorded here so it is not rediscovered.
- **`Expression::infer_name`'s function-call branch returning source text.** Phase 4 stops the *projection* depending on it; other callers are out of scope.

## The `_colN` decision

**Call: synthesize a real alias into the source SQL before printing. Never invent a name at reference time.**

The rule, applied to each top-level select item:

1. Explicit alias → use it, unchanged.
2. No alias, but the CST yields an inferred name that is **a valid bare identifier** (a bare or qualified column ref, or a `CAST` of one) → use it. Every dialect agrees on this name; nothing is spliced.
3. Otherwise (function call, arithmetic, literal, `CASE`, …) → splice ` AS _smelt_col{n}` into the source SQL at that item's end, `n` being the item's 1-based position.

`_smelt_` is already de facto reserved — `wrap_with_type_casts` emits `_smelt_typed` as its derived-table name. Phase 4 makes that explicit: a user projection alias beginning with `_smelt_` is a diagnostic, which is what makes the synthesized name collision-free rather than merely unlikely.

**Why not fail loud on an unnamed column.** Fail-loud discipline targets *unrecognisable* input. `SELECT id, 1 FROM t` is valid SQL that smelt fully understands; we simply declined to name it. Worse, in-model list spread *generates* unaliased literal columns by construction — a diagnostic would kill a shipped feature and blame the user for smelt's own codegen.

**Why not skip the cast for unnamed columns.** That leaves exactly the columns whose backend-native types are least predictable — literals and expressions — unreconciled, contradicting the same-schema-to-every-warehouse promise. It also does nothing for `output_column_names`, whose MERGE column list has the identical hole.

**Why not keep the inferred text name for expressions** (emitting `AS "SUM(x)"` with per-dialect quoting, preserving today's DuckDB-visible names). Rejected on two grounds. It makes a model's output schema depend on source *formatting* — inserting a space inside `SUM( x )` would rename a column, and `output_fingerprint.md` keys projections by name. And the name it preserves is a DuckDB accident that BigQuery and Spark never agreed with, so "preserving" it is really propagating one engine's convention into the portable layer.

**The cost, stated plainly.** An unaliased expression column visible today on DuckDB as `SUM(x)` becomes `_smelt_col2`. That is a user-visible output-schema change on the reference backend, and it is the reason Phase 4 carries the docs-site update and the example-workspace sweep. The project has no backward-compatibility constraint (`CLAUDE.md`), and the alternative is a column name that differs per backend — which the multi-backend spec already promises it does not.

## Progress tracking

| Phase | Status  | Commit | Date |
|-------|---------|--------|------|
| 1     | done    | 284459da | 2026-08-19 |
| 2     | done    | 55eee2b8 | 2026-08-20 |
| 3     | done    |        | 2026-08-20 |
| 4     | pending |        |      |
| 5     | pending |        |      |
| 6     | pending |        |      |

---

## Phase 1 — Spec: the projection derives from the source CST

**Intent.** Land the rule before the code, per the spec-first rule. No production code changes.

**Spec edits.**
- `multi_backend.md` §"Output-schema type conformance": state that the column names and inferred types the cast wrap uses are derived from the model's **source** select list, before dialect lowering, and that the printer's output is never re-read to recover them. State the alias-synthesis rule (the three cases above) as normative surface, including the reserved `_smelt_` prefix.
- `multi_backend.md` §"Whole-row MERGE": correct "derived from the compiled SQL's select list" to the source select list. The existing rationale — "so the build path and the editor agree on what a model's columns are" — is what this makes true; the analyzer's `model_schema` reads source SQL, so a build path reading printed SQL could not have agreed with it by construction.
- `multi_backend.md` §Known Divergences: delete the median entry's closing paragraph ("The general hazard remains recorded here rather than fixed: …"). Keep the measured-bug narrative; it is history worth having.

**Tests (TDD).**
- `cargo test -p smelt-logical --test probe_obligation`-style spec-presence assertion is not appropriate here; instead assert the spec headings this plan depends on exist and carry the new rule, via the existing spec-drift path: run `/smelt:validate multi_backend` and record the drift report in the phase notes. The report must name the implementation as diverging (the code has not moved yet) — that is the red state.

**Review checklist (implementer).**
- [ ] No production code touched.
- [ ] Timeless-oracle rule honored: no phase vocabulary in the spec body.
- [ ] The retired divergence paragraph is deleted, not softened.

**Review checklist (reviewer).**
- [ ] The new rule is stated as behaviour, not as a description of the fix.
- [ ] §"Whole-row MERGE" and §"Output-schema type conformance" agree with each other on where the projection comes from.

**Commit.** `docs: the projection derives from the source select list, not printed SQL`

---

## Phase 2 — One owner for the projection, wired into `compile()`

**Intent.** Introduce a single pure function that derives the projection from the source CST, and route `compile()` through it. The other three entry points follow in Phase 3.

**Shape.** A `Projection` value — the ordered list of `(name, TypedColumn)` — built by one function taking the **pre-print** `SelectStmt` plus the `TypeContext` that `apply_type_casts` already assembles. `apply_type_casts` and `output_column_names` both consume it. Make the signatures take the projection (or the source CST), not a `&str` of printed SQL: the type signature is the durable enforcement, the Phase 6 gate is the backstop.

`wrap_with_type_casts` (`smelt-dialect/src/type_conformance.rs:24`) is pure string wrapping and does not change.

**Tests (TDD).**
1. `crates/smelt-runtime/tests/` — compiling `SELECT d, MEDIAN(val) AS med_val FROM events GROUP BY d` for BigQuery yields `output_columns == ["d", "med_val"]` and a cast wrap naming `med_val`. Red today: the printed ARRAY_AGG form does not read back.
2. The same model compiled for DuckDB, Spark and BigQuery yields **identical** `output_columns` and identical cast-wrap column names. This is the invariant, stated as a test.
3. A model whose source projection is a bare `*` still yields an empty `output_columns` (empty means *unknown*, per §"Whole-row MERGE") — the fail-closed behaviour is preserved, not lost in the refactor.
4. Regression guard for `970ef87a`: a BigQuery median still emits no narrowing cast.

**Review checklist (implementer).**
- [ ] `apply_type_casts` and `output_column_names` no longer call `smelt_parser::parse` on printed SQL.
- [ ] The `TypeContext` assembly (upstream schemas, per-entity sources) is unchanged — bug #3 of `20260417-0.3-regression-triage.md` and the G-01 fractional-aggregate cell must stay fixed.
- [ ] No behaviour change for DuckDB on any example workspace.

**Review checklist (reviewer).**
- [ ] The projection is derived once per compile, not recomputed per consumer.
- [ ] Salsa purity and the run-pipeline-parity rule are untouched.

**Commit.** `fix: derive compile()'s projection from the source CST, not printed SQL`

---

## Phase 3 — The remaining three entry points

**Intent.** Route `compile_with_sql`, `compile_with_sql_and_ephemerals` and `compile_with_ephemerals` through the Phase 2 owner, and delete the printed-SQL parse paths so the pattern cannot come back through a side door.

**Tests (TDD).**
1. One test per entry point asserting dialect-invariant `output_columns` for a model using a lowered construct (`MEDIAN`, `%`, `**`).
2. `compile_with_sql_and_ephemerals` and `compile_with_ephemerals`: the ephemeral-CTE prepend still produces correct `output_columns` — today they re-parse `final_sql` *after* the textual `WITH` merge, so this is the case most likely to regress.
3. The `explain --json` path (`smelt-cli/src/commands/explain.rs:572`) still reports the same columns.

**Review checklist (implementer).**
- [ ] `output_column_names(&str)` and the `&str` form of `apply_type_casts` no longer exist.
- [ ] `rg 'smelt_parser::parse' crates/smelt-runtime/src/compile.rs` returns only source-SQL call sites, each justified by a comment.

**Review checklist (reviewer).**
- [ ] `prepend_ephemeral_ctes` still receives printed SQL (deferred, by design) but nothing downstream of it re-parses.
- [ ] The `require_merge_columns` fail-loud path still triggers for a genuine wildcard projection on BigQuery.

**Commit.** `fix: route every compile entry point through the source-derived projection`

---

## Phase 4 — Alias synthesis and the reserved `_smelt_` prefix

**Intent.** Make `_smelt_col{n}` a bound name instead of an invented reference, per the decision above.

**Shape.** A pass over the source SQL before printing that splices ` AS _smelt_col{n}` into each select item matching case 3. Text splicing at CST text ranges — the same technique `emit_count_preservation_probe_from_body` uses, and legitimate here because the input is source smelt SQL, not printed output. Add a `DiagnosticCode` for a user alias beginning with `_smelt_`, emitted from the analyzer so the editor and the build agree (diagnostic parity rule).

**Tests (TDD).**
1. `SELECT id, 1, 2, 3 FROM raw.users` compiles and **executes** on DuckDB, producing columns `id, _smelt_col2, _smelt_col3, _smelt_col4`. This is the `meta_lists` `list_literal` blocker.
2. The same model produces the same four column names on BigQuery.
3. A bare column ref keeps its own name — `SELECT t.user_id FROM t` yields `user_id`, with no `AS` spliced.
4. A user alias `_smelt_foo` raises the new diagnostic, in both the LSP and the CLI path.
5. The bare-spread forms in `meta_workspace_e2e.rs` and `meta_config_e2e.rs` now work; replace the fold-form workarounds with the bare-spread form those files' headers describe, and delete the "orthogonal defect" notes.
6. `examples/meta_lists` comes off the `KNOWN_UNBUILDABLE` list in `example_builds.rs` if its only remaining blocker was this (its unseeded `raw.users` source may still block it — if so, narrow the entry's reason to the source alone and say so).

**Review checklist (implementer).**
- [ ] The synthesized name is derived from position only — deterministic, formatting-independent.
- [ ] No select item with an explicit alias is touched.
- [ ] `docs-site/` updated: an unaliased expression column is named `_smelt_col{n}`, and `_smelt_` is reserved.

**Review checklist (reviewer).**
- [ ] Example workspaces swept for output-schema changes; any downstream model referencing a previously-inferred expression name is updated, not papered over.
- [ ] The diagnostic fires from the analyzer, so the LSP and CLI agree.
- [ ] The three parked "orthogonal defect" comments are gone, not reworded.

**Commit.** `fix: bind synthesized projection aliases instead of inventing _colN`

---

## Phase 5 — The count-preservation probe sees the real body

**Intent.** Feed `emit_count_preservation_probe_from_body` the pre-wrap model body, and close the test gap that hid its failure.

**Shape.** The probe needs the body *before* the cast wrap. Carry it alongside `CompiledModel.sql` rather than reconstructing it — reconstructing by unwrapping the `_smelt_typed` layer would be a fourth instance of the same mistake.

**Tests (TDD).**
1. The failing test first, at the production input shape: `emit_count_preservation_probe_from_body` applied to a cast-wrapped body must locate the join. This is the test that has never existed, and it is red at `HEAD`.
2. End-to-end: a model with a declared `referential_integrity` closure on DuckDB runs with the delta restriction *applied*, not the widened fallback — assert on the emitted statement, not just on the result.
3. The fail-closed path still fails closed for a body that genuinely has no matching join.

**Review checklist (implementer).**
- [ ] No unwrapping of printed SQL anywhere in the fix.
- [ ] The existing unwrapped-body tests still pass — the function's contract is widened, not swapped.

**Review checklist (reviewer).**
- [ ] The `tracing::warn!` fallback remains for genuine structural misses; the fix removes the *spurious* misses only.
- [ ] Maintenance-plan purity: the probe is still emitter-authored, and the driver still only executes.

**Commit.** `fix: give the count-preservation probe the model body it was meant to read`

---

## Phase 6 — A standing gate, and the docs

**Intent.** Make the invariant checkable so it cannot silently return.

**Shape.** A behavioural gate is the right one: projection identity is dialect-invariant. A test compiles a model exercising every construct the printer lowers — `MEDIAN`, `%`, `**`, `QUALIFY`, date literals, `::` casts, array literals — for DuckDB, Spark and BigQuery, and asserts `output_columns` and the cast-wrap column names are byte-identical across all three. Any reintroduced re-parse of printed SQL breaks it immediately, because lowering is exactly what differs between the three.

Add it to `CLAUDE.md`'s invariant list alongside the existing standing gates, per the "architectural decisions land in specs" rule.

**Tests (TDD).**
1. `cargo test -p smelt-runtime --test projection_dialect_invariance` — the gate itself. Verify it is genuinely load-bearing by reverting Phase 2's change locally and confirming it fails.
2. The gate needs no live warehouse: it asserts on compiled SQL, so it runs per-PR.

**Review checklist (implementer).**
- [ ] The gate is listed in `CLAUDE.md` with its exact command.
- [ ] `docs-site/` pages on model output and multi-backend behaviour reflect the alias rule.

**Review checklist (reviewer).**
- [ ] The gate fails when the invariant is violated (demonstrated, not assumed).
- [ ] `/smelt:validate multi_backend` reports no drift.

**Commit.** `test: pin projection identity as dialect-invariant`

---

## Verification

After every phase is `done`:

```bash
bash .claude/scripts/verify-phase.sh
cargo test -p smelt-runtime --test projection_dialect_invariance
cargo test -p smelt-runtime --test execute_parity
cargo test -p smelt-runtime --test statement_parity
cargo test -p smelt-cli --test maintenance_conformance
cargo test -p smelt-dialect --test median_lowering
```

The BigQuery leg (`cargo test -p smelt-cli --test maintenance_conformance_bigquery`) needs a human-minted token — see `docs/handoffs/2026-08-16-bigquery-backend.md`. Ask the user before attempting it.

Then update `docs/ROADMAP.md` with the completion date.

## References

- `docs/specs/multi_backend.md` §"Output-schema type conformance", §"Whole-row MERGE", §Known Divergences
- `docs/specs/diagnostics.md` — the code catalogue
- `docs/research/20260816-bigquery-backend.md` — the live probes behind the median lowering
- `crates/smelt-dialect/tests/median_lowering.rs` — the round-trip test whose doc comment names this bug
