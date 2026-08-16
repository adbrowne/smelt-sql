# Phase 6 plan — runtime lowering for the repair family

## Objective

Make an admitted `Technique::PerGroupRecompute` cell actually execute: resolve it live from the
derived plan, build the emitter's two string inputs, run `emit_per_group_recompute`'s
`StatementGroup` through the backend, and prove the executed SQL is byte-identical to the
emitter's output. Advances criteria 1 (the derived repair cell now *runs* instead of being
inert), 3's executed-vs-emitted half for the repair family, and 6.

## Spec delta

No user-visible surface change — the spec already describes the repair family's semantics
(§"The repair family", landed phase 1). One edit only: `docs/specs/incremental_models.md`
§Known Divergences, narrow the repair-family entry from "runtime lowering and the
executed-vs-emitted parity leg remain" to name only the `diff_patch` write-pin routing (phase 7)
as outstanding.

## Tests

New file `crates/smelt-runtime/tests/repair_lowering.rs`:

1. `resolve_live_per_group_recompute_cell_finds_the_admitted_repair_cell` — a keyed model with a
   non-invertible combiner over a `MutableSnapshot` source resolves `Some((source, cell, key,
   slice))`; the key is the cell's `RowIdentity::Key`, verbatim.
2. `resolve_live_per_group_recompute_cell_none_for_an_append_only_model` — an ordinary
   append-only fold model resolves `None` (repair never displaces an admitted fold cell).
3. `resolve_live_per_group_recompute_cell_fails_loud_on_whole_row_identity` — a synthetic cell
   carrying `RowIdentity::WholeRow` errors by name rather than silently widening to a full repair.
4. `affected_keys_select_bounds_the_read_with_the_cells_scan_clamp` — pure string builder: the
   affected-keys SELECT carries the clamp predicate on the source; an unclamped scan yields the
   unpredicated read (and is only reachable where admission already proved the slice).
5. `per_group_recompute_repairs_only_the_affected_group` — real DuckDB: after a retraction in one
   group, that group's stored value is repaired and an untouched group's row is bit-identical.
6. `per_group_recompute_matches_full_refresh_after_retraction` — `multiset_equal` against a
   full-refresh oracle over the same inputs.

Extend `crates/smelt-runtime/tests/statement_parity.rs`:

7. `per_group_recompute_statements_come_from_the_emitter` — `RecordingBackend` over a real
   `execute_project` run; the recorded `StatementGroup` is byte-equal to a direct
   `emit_per_group_recompute` call with the batch's own inputs, plus the family's `multiset_equal`
   result-equivalence leg.

Extend `crates/smelt-runtime/tests/diagnostics.rs`:

8. `per_group_recompute_preview_renders_statements_for_an_admitted_repair_cell` — the technique
   preview arm no longer returns the "no live statement builder yet" error.

## Tasks

1. `maintenance_driver.rs`: add `resolve_live_per_group_recompute_cell(sql, table, metadata,
   sources, explicitly_mutable, technique_overrides)` — derives the plan exactly once (purity
   rule), scans `NewData`/`UpstreamMutation` cells for `Technique::PerGroupRecompute`, applies the
   same `unaddressed_technique_pin` + `resolve_cell_choice` gating the sibling resolvers use, and
   returns the cell with its proven key and `ScanClamp`. `RowIdentity::WholeRow` on a
   `PerGroupRecompute` cell is a fail-loud `bail!`, never a skip.
2. `maintenance_driver.rs`: pure string builders `repair_affected_keys_select(source_table, key,
   clamp)` and `repair_candidate_select(full_model_sql, key, affected_keys_select)` — plain
   `SELECT` text the emitter consumes, per that module's "callers resolve strings, emitters
   assemble" contract. Candidate is the model's FULL (unwindowed) recompiled SQL semi-joined to the
   affected keys, so a group is recomputed whole; the clamp is pushed into the affected-keys read
   only (see the outcome's decision log for why not onto the output wrapper).
3. `maintenance_driver.rs`: `execute_per_group_recompute(backend, schema, table, key,
   affected_keys_select, candidate_select, retry)` — mirrors `execute_staged_membership_recompute`:
   emitter → `retry_backend_call` → `Backend::execute_statement_group`, returning `ExecutionResult`.
4. `execute.rs`: route the repair cell at the existing incremental technique ladder, after the
   column-scoped-merge and membership-recompute sites and before the region-recompute default. A
   cell that resolves live but whose inputs cannot be built errors by name rather than falling back.
5. `diagnostics.rs`: replace the `Technique::PerGroupRecompute` fail-loud arm with a real builder
   over the same task-2 helpers (its "no live statement builder yet" comment is now false).
6. Add the `statement_parity.rs` leg and update that file's module doc comment to name the repair
   family alongside the region/keyed-fold/column-scoped families.
7. Verify the structural no-authoring leg still covers the new statement shapes; extend its scan
   patterns if the repair `DELETE ... USING` shape is not already matched.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test repair_lowering --test statement_parity --test
  technique_lowering --test diagnostics`
- `cargo test -p smelt-cli --test maintenance_conformance --test explain --test explain_model`
- `cargo test -p smelt-logical --test walk_coverage` (no new whole-text scan)

## Commit message

`feat(incremental): lower per-group recompute cells at runtime with emitter parity`
