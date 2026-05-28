# Plan: alias column lists for `(VALUES …)` and CTE derived tables

**Date**: 2026-05-28
**Spec**: [`docs/specs/types.md`](../specs/types.md), [`docs/specs/scoping.md`](../specs/scoping.md)
**Spec diff**: `types.md` §Surface gains a "VALUES-derived tables" subsection specifying column-wise type unification across rows; `scoping.md` §"Upstream model and source schemas" gains a clause for alias column lists (`AS t(c1, c2, …)` and `WITH cte(c1, c2, …) AS …`); `types.md` §Known Divergences removes the silent-`Unknown` entry for VALUES derived tables.
**Tracking PR / branch**: branch `worktree-unknown_types`.
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/types.md` §Surface and §"Strict-by-default doctrine"; read `docs/specs/scoping.md` §"Upstream model and source schemas". These are the correctness oracle.
2. Confirm you are on branch `worktree-unknown_types`. If not, ask before continuing.
3. Find the next `pending` phase in the Progress table. If all are `done`, run Verification and stop.

**Per-phase loop (`/smelt:implement`):** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**
- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule (update the spec via `/smelt:spec` first).
- The parser change requires a new syntax kind that conflicts with existing ones — flag before adding.
- The fix shape grows beyond `smelt-parser` (CST/AST) and `smelt-db` (typing) — flag.

**Conventions every phase:**
- Red-green TDD; typing tests drive the *real* `model_function_type` / `typed_model_schema` query, not a sub-helper. Parser tests drive the *real* `parse_file` and assert on the produced CST.
- Real-fixture coverage: `examples/per_cohort_union/models/orders.sql` is the existing reproducer; verify it through `example_diagnostics` and `example_workspaces`.
- Atomic per-phase commit using the `Commit.` line verbatim; push after each.
- Honor `CLAUDE.md` invariants: `smelt-db` pure-function rule; the Workspace Loading Parity Rule (no LSP-only or CLI-only paths).
- **Timeless-oracle rule.** Phase vocabulary lives in this plan only. Spec / `docs-site` edits describe the feature as if it has always existed. As each gap closes, delete its §Known Divergences entry in the same commit.

---

## Context

`examples/per_cohort_union/models/orders.sql`:

```sql
SELECT id, user_id, region, revenue, created_at
FROM (VALUES
    (1, 10, 'us-west-2', 150, CAST('2024-01-01' AS TIMESTAMP)),
    ...
) AS t(id, user_id, region, revenue, created_at)
```

`target/debug/smelt type --project-dir examples/per_cohort_union` reports every column as `UNKNOWN`. The bug is layered across parser, AST, and type inference:

1. **Parser**: `crates/smelt-parser/src/parser/select.rs:423-433` consumes `AS t` but never looks for `LPAREN` to consume the alias column list. The `(id, user_id, …)` tokens are dropped — there is no CST node carrying them.
2. **AST**: `crates/smelt-parser/src/ast.rs:2566-2583` exposes `Subquery::select_stmt()` but no `Subquery::values_clause()`. The grammar already wraps `(VALUES …)` in a `SUBQUERY` CST node (`crates/smelt-parser/src/parser/select.rs:336-341`), so the data is there; no accessor reads it.
3. **Type inference**: no VALUES row-typing machinery exists in `crates/smelt-db/src/type_inference/` or `crates/smelt-db/src/queries/schema.rs`. The only mention of `VALUES` in `smelt-db` (`hof.rs:1244`) concerns meta-language spread admissibility, not row typing.
4. **Schema integration**: `crates/smelt-db/src/queries/schema.rs:869-904` (`process_table_ref_pure`'s subquery branch) only handles `subquery.select_stmt()`. For `(VALUES …) AS t`, `select_stmt()` returns `None`, the alias is never registered, and the outer `SELECT id, user_id, …` falls through to `Unknown`.

The downstream `all_cohorts_unioned` model and the generator-emitted cohort models all inherit `Unknown` from this root cause.

## Scope

### In scope (spec coverage)
- `types.md` §Surface gains a "VALUES-derived tables" subsection: column-wise unification of element types across rows via the existing promotion lattice; empty VALUES (no rows) emits a diagnostic — never silently produces `Unknown`.
- `scoping.md` §"Upstream model and source schemas" gains a clause: an alias column list — written as `AS t(c1, c2, …)` on a derived table, or as `WITH cte(c1, c2, …) AS (…)` on a CTE — rebinds the underlying relation's columns; arity must match; when omitted, the underlying column names are used (for SELECT-subqueries / CTEs) or `col1..colN` (for VALUES without an alias list).
- A new diagnostic `AliasColumnArityMismatch` (covers both VALUES and CTE forms) flags arity mismatches.

### Explicitly out of scope
- VALUES outside the derived-table position (e.g. bare `VALUES (1,2)` as a top-level statement, or in `INSERT … VALUES`). Not exercised by the example workspace; defer.
- `LATERAL (VALUES …)` and other lateral forms. Defer.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | 86f755fe | 2026-05-28 |
| 2     | done     | 01fc027f | 2026-05-28 |
| 3     | done     |        | 2026-05-28 |

---

### Phase 1: Parser captures alias column lists (VALUES + CTE); AST exposes VALUES clause

**Goal.** The parser captures alias column lists in *both* surface positions where they appear today — `AS t(c1, c2, …)` on a derived table and `WITH cte(c1, c2, …) AS (…)` on a CTE — as a shared `ALIAS_COLUMN_LIST` syntax kind. The AST exposes the new accessor uniformly and `Subquery::values_clause()` returns the contained `ValuesClause` for VALUES subqueries.

**Pre-conditions.** None.

**TDD tests to write first.**

1. `crates/smelt-parser/src/parser/tests.rs` — parsing `SELECT * FROM (VALUES (1)) AS t(c)` produces a CST containing an `ALIAS_COLUMN_LIST` node with a single IDENT `c`. Parsing without the list (`AS t`) produces no `ALIAS_COLUMN_LIST` node and no parse errors. Parsing `AS t(a, b, c)` produces three IDENTs.
2. `crates/smelt-parser/src/parser/tests.rs` — parsing `WITH cte(a, b) AS (SELECT 1, 2) SELECT * FROM cte` produces a CST with an `ALIAS_COLUMN_LIST` node carrying `a` and `b` *under the CTE binding*, not under the inner SELECT's table refs. Parsing `WITH cte AS (SELECT 1) SELECT * FROM cte` produces no `ALIAS_COLUMN_LIST` node.
3. `crates/smelt-parser/src/parser/tests.rs` — `Subquery::values_clause()` returns `Some(ValuesClause)` for `(VALUES (1, 2))` and `None` for `(SELECT 1)`. The existing `parse_spread_in_values_row` test continues to pass.
4. `crates/smelt-parser/src/parser/tests.rs` — `TableRef::alias_column_names()` returns `Some(vec!["id", "user_id", …])` for the `examples/per_cohort_union/models/orders.sql` body, and `None` for an alias without a column list. A symmetric accessor on the CTE AST wrapper (`Cte::column_names()` or equivalent — match the existing CTE wrapper's naming) returns the declared names.

**Implementation shape.**
- Add `ALIAS_COLUMN_LIST` to `crates/smelt-parser/src/syntax_kind.rs`.
- In `crates/smelt-parser/src/parser/select.rs` `parse_table_ref`, after the alias is consumed (lines 423-433), if the next non-trivia token is `LPAREN`, start an `ALIAS_COLUMN_LIST` node, consume `ident (, ident)*`, expect `RPAREN`, finish the node. Recovery: on any unexpected token, finish the node and let the outer error path handle it.
- In the CTE parser path (`select.rs:832-878`), wrap the existing flat IDENT consumption in the same `ALIAS_COLUMN_LIST` node — same recovery rules.
- Add `ValuesClause` AST wrapper in `crates/smelt-parser/src/ast.rs` if not already present (the grammar already produces a `VALUES_CLAUSE` node via `parse_values_clause` at `expr.rs:773-813`; confirm and reuse).
- Add `Subquery::values_clause(&self) -> Option<ValuesClause>` accessor next to the existing `select_stmt()`.
- Add `TableRef::alias_column_names(&self) -> Option<Vec<String>>` and the CTE-side accessor (match the existing CTE AST wrapper's naming).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-parser/src/syntax_kind.rs` — new kind.
- `crates/smelt-parser/src/parser/select.rs` — both alias-list parsing sites.
- `crates/smelt-parser/src/ast.rs` — `ValuesClause` wrapper, `Subquery::values_clause`, `TableRef::alias_column_names`, CTE column-name accessor.
- `crates/smelt-parser/src/parser/tests.rs` — TDD tests.

**Docs touched.** None this phase — type-system behavior is unchanged until Phase 2.

**Review checklist (material findings only):**
- [ ] CST contains `ALIAS_COLUMN_LIST` only when an alias column list was written; no extra nodes inserted on the no-list path; same kind used for VALUES and CTE positions.
- [ ] `Subquery::values_clause()` does not regress existing `select_stmt()` consumers.
- [ ] Parser error recovery: a malformed `AS t(a,)` or `WITH cte(a,) AS (…)` does not panic and produces a diagnostic at the offending span.
- [ ] No new clippy warnings.

**Commit.** `feat(parser): capture VALUES and CTE alias column lists as a shared CST node`

---

### Phase 2: Type inference for VALUES derived tables and CTE column-list rebinding

**Goal.** A model whose FROM clause is `(VALUES …) AS t[(c1, c2, …)]` produces a typed schema with each column's type computed by unifying the corresponding row-element types across all VALUES rows via the existing promotion lattice. A CTE declared with `WITH cte(c1, c2, …) AS (SELECT …)` rebinds the inner SELECT's columns under the declared names. When an alias column list is omitted: CTEs and SELECT-subqueries use the underlying SELECT's column names; VALUES uses `col1..colN`. An empty VALUES (no rows) is a diagnostic, not silent `Unknown`.

**Pre-conditions.** Phase 1 done.

**TDD tests to write first.**

1. `crates/smelt-db/tests/values_derived_table_typing.rs` (new integration test). Cases:
   - Single-row, fully concrete: `(VALUES (1, 'a', CAST(NULL AS DATE))) AS t(i, s, d)` → `{i: Integer, s: Text, d: Date}` (or `Varchar`, per the existing `Text`/`Varchar` compatibility — assert via the existing comparison helper if one exists, otherwise pin to whichever the existing literal-inference returns).
   - Multi-row promotion: `(VALUES (1), (2.0)) AS t(x)` → `{x: Double}`.
   - No alias column list: `(VALUES (1, 2)) AS t` → `{col1: Integer, col2: Integer}`.
   - The full reproducer: `(VALUES (1, 10, 'us-west-2', 150, CAST('2024-01-01' AS TIMESTAMP)), …) AS t(id, user_id, region, revenue, created_at)` → all five columns concrete.
2. `crates/smelt-db/tests/cte_column_list_rebinding.rs` (new integration test). Cases:
   - `WITH cte(a, b) AS (SELECT 1, 2.0) SELECT a, b FROM cte` → `{a: Integer, b: Double}`.
   - No CTE column list: `WITH cte AS (SELECT 1 AS x) SELECT x FROM cte` → `{x: Integer}` (existing behavior; regression assertion).
   - Mixed types preserved on rebinding: `WITH cte(price) AS (SELECT CAST(1.5 AS DOUBLE)) SELECT price FROM cte` → `{price: Double}`.
3. `crates/smelt-cli/tests/example_diagnostics.rs` — `examples/per_cohort_union/` typed-schema path: `orders.sql` exports five concrete-typed columns; `all_cohorts_unioned.sql` and the generator-emitted cohort models inherit concrete types.
4. Regression: `example_diagnostics` (currently 75) and `example_workspaces` (21) stay green; no existing CTE-using example regresses.

**Implementation shape.**
- New pure helper in `crates/smelt-db/src/type_inference/` (a new `values.rs` module, or adjacent to `dispatch.rs`):
  ```rust
  fn infer_values_columns(values: &ValuesClause, ctx: &TypeContext) -> Result<Vec<TypedColumn>, ValuesError>
  ```
  Iterates VALUES rows, infers each element's `DataType` via the existing expression-inference entry point, unifies column-wise across rows using the existing `promote_types` lattice. For an empty VALUES (no rows), return `Err(ValuesError::Empty)` — the schema-integration site surfaces this as a diagnostic (Phase 3).
- In `crates/smelt-db/src/queries/schema.rs:869-904` (subquery branch of `process_table_ref_pure`), branch on `subquery.values_clause()` in addition to `select_stmt()`. When VALUES is present: call `infer_values_columns`; determine column names — `table_ref.alias_column_names()` when present, otherwise `col1..colN`; bind into the `TypeContext` and register the alias.
- For CTE rebinding: locate the existing CTE typing site (Salsa-direct or the same `build_type_context` pass — confirm during investigation). When the CTE AST wrapper exposes a column-name list (added in Phase 1), rebind the inner SELECT's column types to the declared names *positionally*; arity-mismatch is surfaced as a diagnostic in Phase 3. When the column list is absent, behavior is unchanged.
- Keep the helper pure; Salsa queries stay thin wrappers.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/src/type_inference/` — the new VALUES helper module.
- `crates/smelt-db/src/queries/schema.rs` — subquery-branch extension and CTE-rebinding hook.
- `crates/smelt-db/tests/values_derived_table_typing.rs`, `crates/smelt-db/tests/cte_column_list_rebinding.rs` — new integration tests.
- `crates/smelt-cli/tests/example_diagnostics.rs` — typed-schema assertion for `per_cohort_union`.

**Docs touched.**
- `docs/specs/types.md` — add §Surface subsection "VALUES-derived tables" with the column-wise unification rule; remove the silent-`Unknown` entry from §Known Divergences if it names VALUES.
- `docs/specs/scoping.md` — §"Upstream model and source schemas" gains a clause on alias column lists.
- `docs-site/docs/reference/language.md` — short note on VALUES-derived-table typing and the alias column list.

**Review checklist (material findings only):**
- [ ] TDD tests drive the real `model_function_type` / `typed_model_schema` query.
- [ ] `smelt-db` pure-function rule preserved (no Salsa inside the helper).
- [ ] `example_diagnostics` + `example_workspaces` stay green; `per_cohort_union` now reports concrete types.
- [ ] Promotion across heterogeneous rows uses the existing lattice — no new ad-hoc rules.
- [ ] Spec + user-doc edits are timeless (no `Phase X`).

**Commit.** `feat(types): infer VALUES derived-table column types with alias column list`

---

### Phase 3: Arity-mismatch and empty-VALUES diagnostics

**Goal.** When the alias column list count does not match the underlying relation's column count — for either a VALUES derived table or a CTE — emit a single diagnostic at the alias-list span. An empty VALUES (no rows) emits a separate diagnostic. No silent fallback in either case.

**Pre-conditions.** Phases 1–2 done.

**TDD tests to write first.**

1. `crates/smelt-db/tests/values_derived_table_typing.rs` — `(VALUES (1, 2)) AS t(a)` and `(VALUES (1)) AS t(a, b)` each emit exactly one `AliasColumnArityMismatch` diagnostic.
2. `crates/smelt-db/tests/cte_column_list_rebinding.rs` — `WITH cte(a) AS (SELECT 1, 2) SELECT * FROM cte` and `WITH cte(a, b) AS (SELECT 1) SELECT * FROM cte` each emit exactly one `AliasColumnArityMismatch` at the CTE column-list span.
3. `crates/smelt-db/tests/values_derived_table_typing.rs` — `(VALUES) AS t` (or whichever lexically valid empty form the parser accepts) emits exactly one `EmptyValuesClause` (final name TBD; pick the one that matches the existing naming convention).
4. Regression: previously-passing cases (no alias list, matching alias list, non-empty VALUES) emit zero of these diagnostics.
5. New broken-fixtures: `examples/values_broken_alias_arity/` and `examples/cte_broken_alias_arity/` each emit exactly `AliasColumnArityMismatch`; `crates/smelt-cli/tests/example_diagnostics.rs` (broken-workspace path, one-code-per-fixture pattern) asserts.

**Implementation shape.**
- Mint `DiagnosticCode::AliasColumnArityMismatch` and `DiagnosticCode::EmptyValuesClause` in `crates/smelt-db/src/lib.rs` / `diagnostics_types.rs`.
- In `infer_values_columns`, propagate the empty case as a structured error; the schema-integration site catches it and emits `EmptyValuesClause` at the VALUES span.
- In the schema-integration site (or shared helper if both VALUES and CTE rebinding go through one), compare alias-list length to underlying column count when an alias list is present; on mismatch, push `AliasColumnArityMismatch` at the alias-list span and proceed with the narrower of the two counts (avoid cascading errors).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/src/lib.rs` / `crates/smelt-db/src/diagnostics_types.rs` — new codes.
- `crates/smelt-db/src/type_inference/` or `crates/smelt-db/src/queries/schema.rs` — emission sites.
- `crates/smelt-db/tests/values_derived_table_typing.rs`, `crates/smelt-db/tests/cte_column_list_rebinding.rs` — diagnostic tests.
- `crates/smelt-cli/tests/example_diagnostics.rs` — broken-fixture tests.
- `examples/values_broken_alias_arity/`, `examples/cte_broken_alias_arity/` — the broken fixtures.

**Docs touched.**
- `docs/specs/types.md` §Diagnostic codes — add `AliasColumnArityMismatch` and `EmptyValuesClause` (timeless phrasing).
- `docs/specs/scoping.md` — note that arity mismatch is a diagnostic, not silently absorbed.
- `docs-site/docs/reference/diagnostics.md` (if it exists) — add entries.

**Review checklist (material findings only):**
- [ ] Diagnostic fires at the alias-list span (or VALUES span for empty), not at an enclosing model name.
- [ ] No cascading `Unknown` produced as a side effect; downstream columns still type as best they can with the truncated alias.
- [ ] `example_diagnostics` + `example_workspaces` stay green; the new broken fixtures each emit exactly the expected code.
- [ ] Spec + user-doc edits are timeless.

**Commit.** `feat(types): diagnose alias-column arity mismatch and empty VALUES`

---

## Deferred during implementation

(Append-only.)

- `LATERAL (VALUES …)` and lateral derived tables in general.
- Bare top-level `VALUES (1, 2)` and `INSERT … VALUES`.

## Verification

- `target/debug/smelt type --project-dir examples/per_cohort_union` shows no `UNKNOWN` for `orders`, the cohort generators' emissions, or `all_cohorts_unioned`.
- `cargo test -p smelt-cli --test example_diagnostics` — green (incl. the new broken fixture if Phase 3 lands).
- `cargo test -p smelt-lsp --test example_workspaces` — green.
- `cargo fmt --all -- --check`, `cargo clippy --all-targets`, `cargo test` — green.
- `/smelt:validate types` — no drift on the new §Surface subsection or §Diagnostic codes entry.
- `/smelt:validate scoping` — no drift on the alias-column-list clause.
