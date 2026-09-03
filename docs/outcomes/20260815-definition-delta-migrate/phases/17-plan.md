# Phase 17 — Maintained-model creation technique; frontmatter-time `grain: key` identity check

## Objective

Close three residues at once. (a) The partition-addressed **maintained-model creation cell**
(`Trigger::NewData { source: <upstream model> }`, `Technique::DeleteInsert`) gains a real
execution technique: `resolve_incremental_strategy` becomes edge-aware and reads *that* cell,
and a driving edge refused `ReachNotDerivable` stops silently region-recomputing. (b) A
`grain: key` model that derives identity from its own `GROUP BY` (no top-level `unique_key:`)
is checked at frontmatter/diagnostic time, not only at plan derivation. (c) The `GROUP BY`
key derivation stops returning an empty key for a column named `order_id` — a keyword/substring
collision that silently breaks `grain: key` admission (phase 13 summary). Advances the outcome's
success criteria on plan-consumer completeness and on `incremental_models.md` Known-Divergence
closure.

## Root cause of (c)

`analysis::mod::find_keyword_not_in_parens`'s word-boundary check uses `is_ascii_alphanumeric`,
which does not include `_`. Scanning `" ORDER_ID"` for the `GROUP BY` end-keyword `ORDER`
matches at byte 1 (`' '` before, `'_'` after — both "non-alphanumeric"), so
`extract_group_by_from_text` truncates the clause to the empty string and `group_by_unique_key`
returns `[]`. Every end keyword is affected (`having_flag`, `union_all`, `limit_count`,
`except_code`, `intersect_key`, `fetch_size`). Single caller, so blast radius is contained.

## Spec delta (made first, by the implement step)

- `docs/specs/models.md` §"Constraint violations": add a row — *`grain: key` asserted with no
  declared `unique_key:` and no `GROUP BY`-derivable identity in the model's own SQL → hard
  error naming the asserted grain and the empty derived key.*
- `docs/specs/models.md` §Known Divergences: delete the "narrow gap" clause ("a `grain: key`
  model that declares no top-level `unique_key:` … is not yet checked against that GROUP-BY-
  derived key at the frontmatter level").
- `docs/specs/incremental_models.md` §Known Divergences: delete the bullet "**Frontmatter-time
  grain checking has one narrow gap**" and the bullet "**No execution technique keys off a
  maintained-model creation cell**"; if the refusal leg lands narrowed (see Tasks), restate the
  residue honestly rather than deleting.
- `docs/specs/incremental_models.md` §"Upstream model edges": one sentence — the partition-
  addressed creation cell's admitted technique is what the run loop executes, and an edge
  refused `ReachNotDerivable` with no other creation cell refuses the run.
- `docs-site/docs/` — the page carrying the `grain:` assertion rules: one sentence for the new
  constraint violation.

## Tests (red first)

- `smelt-logical` unit (`analysis/mod.rs`): `group_by_column_prefixed_by_an_end_keyword_survives`
  — table-driven over `order_id`, `having_flag`, `union_all`, `limit_count`, `except_code`,
  `intersect_key`, `fetch_size`: each is derived as the sole GROUP BY key.
- `smelt-logical` unit: `real_order_by_after_group_by_still_terminates_the_clause` — the fix does
  not stop `ORDER BY` / lowercase `order by` / `HAVING` from ending the clause.
- `smelt-logical` unit: `quoted_or_qualified_end_keyword_is_not_a_clause_terminator` — `t.order`
  and `"order"` as GROUP BY expressions.
- `crates/smelt-logical/tests/declared_unique_key_classifier.rs`:
  `group_by_unique_key_derives_order_id` — `group_by_unique_key` on `GROUP BY order_id` returns
  `["order_id"]`, and `declared_unique_key_matches(["order_id"], sql)` is `Ok`.
- `crates/smelt-db/tests/` (maintenance suite): `grain_key_over_order_id_derives_a_keyed_plan` —
  the plan's `PlanGrain::Key` carries `["order_id"]`, not `[]`.
- `crates/smelt-db/tests/` (diagnostics): `grain_key_without_unique_key_or_group_by_errors` →
  `DiagnosticCode::GrainAssertionMismatch`, message naming the asserted grain and the empty
  derived key; `grain_key_without_unique_key_but_with_group_by_is_clean` (no diagnostic).
- `crates/smelt-lsp` parity: the same model surfaces the diagnostic through the LSP backend
  (extend the existing maintenance-diagnostic parity test rather than adding a file).
- `crates/smelt-runtime/tests/model_edge_creation_cell.rs` (new):
  - `model_edge_creation_cell_drives_the_incremental_strategy` — a two-model chain (clocked
    maintained upstream) resolves its strategy from the *model edge's* `Trigger::NewData` cell,
    asserted on a fixture where that cell and the source-driven cell differ.
  - `clockless_maintained_upstream_refuses_instead_of_silently_region_recomputing` — a maintained
    upstream with no `timeseries:` and no `KeyedUpsert` shape, and no other creation cell, is a
    fail-loud run refusal naming the edge and `ReachNotDerivable`.
  - `clockless_upstream_alongside_a_clocked_source_still_runs` — the refusal is narrow.

## Tasks

1. Fix `find_keyword_not_in_parens`'s boundary predicate: an identifier char is
   `is_ascii_alphanumeric() || b'_'`; additionally reject a match preceded by `.`, `"` or a
   backtick. Doc-comment the rule. Red-green against the unit tests above.
2. Re-run `cargo test -p smelt-logical -p smelt-db --quiet` and inspect any newly-changed
   expectations — a test that encoded the truncation as correct must be corrected, not relaxed.
3. In `crates/smelt-db/src/queries/maintenance.rs`, extend the existing `ConfigGrain::Key` arm's
   identity check: when `metadata.unique_key` is `None`, `metadata.grain == Some(Key)`, and
   `derive_group_by_unique_key(sql)` is empty, emit `GrainAssertionMismatch` through the same
   diagnostic seam the plan-derivation refusals already use, so it reaches `file_diagnostics()`
   (CLI + LSP parity) and `smelt explain` without a run. Do not re-derive the key.
4. Add `model_edges: &[ModelEdge]` to `resolve_incremental_strategy`; when non-empty, derive via
   `derive_model_maintenance_plan_with_edges` (the same edge-aware derivation
   `resolve_live_delta_restriction_facts` uses — never a second derivation) and prefer
   `plan.cell_for(&Trigger::NewData { source: <driving edge> })` over the first-`NewData` match.
5. Return a fail-loud refusal (not `backend_default`) when the driving edge's creation cell is
   absent *and* `plan.refusals` names it `ReachNotDerivable` *and* the plan has no other
   `Trigger::NewData` cell. Surface it as a run error naming the edge; no new `DiagnosticCode`.
6. Thread `model_edges` at the `execute.rs:2912` call site (`model_edges_for` is already computed
   a few lines below — hoist it above the strategy resolution rather than computing it twice).
7. Run `cargo test -p smelt-cli --test example_diagnostics` and the `examples/web_analytics`
   fixtures. **If a real fixture newly refuses**, narrow the refusal condition further rather
   than editing the fixture, and record the narrowing in the phase summary + spec bullet.
8. Land the spec delta and the docs-site sentence.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --lib analysis --quiet`
- `cargo test -p smelt-db --test maintenance_model_upstream --quiet`, plus the smelt-db
  maintenance diagnostics suite
- `cargo test -p smelt-runtime --test model_edge_creation_cell --test statement_parity --test key_addressed_model_edge_lowering --quiet`
- `cargo test -p smelt-cli --features duckdb --test maintenance_conformance --test example_diagnostics --quiet`
- `cargo test -p smelt-lsp --test example_workspaces --quiet`

## Commit

`fix(maintenance): dispatch the maintained-model creation cell; check GROUP BY-derived grain: key identity at frontmatter time; fix the ORDER_ID keyword collision`
