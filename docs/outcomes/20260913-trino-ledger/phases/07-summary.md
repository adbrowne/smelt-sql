# Phase 7 summary — The staged relation group without temp tables

**Shipped:**
- `BackendCapabilities` gains `staged_relation_residence` (`StagedRelationResidence::
  SessionTemporary` | `TargetSchema`) and `staged_relation_group_is_atomic: bool`
  (`crates/smelt-dialect/src/dialect.rs`), set in all five constructors: `SessionTemporary`/`true`
  everywhere except `trino_iceberg()`, which is `TargetSchema`/`false`.
- New single-owner module `crates/smelt-logical/src/maintenance/emit/staged_relation.rs`:
  `StagedRelation { name, residence, atomic }`, `StagedRelation::derive(purpose, qualified_table,
  residence, atomic)`, `session_temporary(name)`, `create_prefix()`, `reclaim_statement()`,
  `drop_statement()`.
- The four staged emitters re-keyed from `staged_relation: &str` to `&StagedRelation`:
  `emit_staged_candidate_conditional`, `emit_staged_candidate_conditional_recompute`,
  `emit_per_group_recompute`, `emit_diff_patch` (`emit/staged.rs`, `emit/recompute.rs`). Each now
  derives its `CREATE` spelling from `create_prefix()`, prepends `reclaim_statement()` when
  `Some`, and sets `StatementGroup::transactional` from `staged_relation.atomic` instead of a
  hardcoded `true`. `emit_staged_candidate_conditional_keyless` (whole-row) is unchanged — no
  production Trino caller reaches it and it is out of this phase's four-emitter scope.
- Every ad hoc name site re-keyed to the deriver: `cumulative.rs:216`,
  `delta_restriction/mod.rs:75`, and `repair/execute.rs`'s `repair_staged_relation`/
  `diff_patch_staged_relation` (now return `StagedRelation`, not `String`).
- Live proof: `crates/smelt-backend-trino/tests/staged_relation_lifecycle.rs` — runs the
  lifecycle (reclaim, CREATE, INSERT, interrupt, reclaim again, re-stage, apply, DROP) directly
  through `TrinoBackend` against a live coordinator, asserting the target is byte-unchanged after
  the interruption and correct after the recovered re-run, with no orphan relation surviving.
- Fail-loud gate: `crates/smelt-runtime/tests/staged_relation_atomicity.rs` —
  `no_non_atomic_backend_is_handed_a_transactional_group` asserts all four re-keyed emitters emit
  `transactional == false` for a non-atomic `StagedRelation` and `true` for the atomic shape.
- `smelt-dialect`: `every_capability_profile_declares_a_staged_relation_residence` plus inline
  assertions in `every_flag_matches_matrix`; exhaustiveness pattern in `all_fields_destructured`
  extended.
- Spec deltas: `multi_backend.md` (matrix rows split, §"Column-scoped merge and conditional-write
  capabilities" rewritten, Trino paragraph extended), `model_transforms.md` (§"The staged-candidate
  conditional DELETE+INSERT" restated + non-atomic recovery obligations), `state.md` (§"The
  degradation contract" gains the atomicity-claim paragraph), `docs-site/docs/guide/targets.md`
  (new Trino limitations bullet for `staged_relation_group_is_atomic`).

**Decisions:**
- Only 4 of the 5 `emit/staged.rs`+`emit/recompute.rs` functions were re-keyed
  (`emit_staged_candidate_conditional_keyless` excluded) — matches the plan's literal "four staged
  emitters" count; keyless has no production Trino caller path today and its sentinel relation
  (a second hardcoded temp table) is out of this phase's scope.
- Production call sites (all four ad hoc name sites) construct `StagedRelation` with
  `SessionTemporary`/`true` unconditionally rather than threading `BackendCapabilities` through —
  correct because no `MaintenanceDialect::Trino` variant exists (phase 4's standing decision), so
  no live Trino path reaches these callers yet; DuckDB/Spark/BigQuery names and behavior are
  byte-unchanged.
- `large-file-baseline.txt`: bumped 3 files 2-5 lines each (`cumulative.rs`, `repair_lowering.rs`,
  `technique_lowering/basic.rs`) — one-line call-site wraps, not new abstractions. Ran
  `--update` once, which destroyed prior phases' sign-off comment history; reverted via
  `git checkout` and hand-edited the 3 numbers instead, preserving history. **For future phases:
  never run `large-file-check.sh --update` bare — it regenerates the whole file and drops every
  previously-appended sign-off note; hand-edit the specific line(s) instead.**

**For the next planner:**
- Phase 8 (locking and versioning) and phase 9 (`.smelt/` non-correctness) are next, both pending.
- `emit_staged_candidate_conditional_keyless`'s sentinel relation is still a hardcoded
  `CREATE TEMP TABLE` — if a future phase needs the keyless path live on Trino, it will need the
  same residence/atomicity treatment this phase gave the primary staged relation.
- The live lifecycle test stands in for "apply" with a single INSERT rather than a real
  maintenance-emitter statement, since no `MaintenanceDialect::Trino` exists — once T4
  (`20260913-trino-incremental`) lands real DELETE+INSERT statement spellings for Trino, this test
  could be extended (or a sibling added) to run the real emitted group end to end.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full workspace `cargo test`, example_diagnostics).
- `cargo test -p smelt-logical --lib maintenance::emit::` — 91 passed.
- `cargo test -p smelt-dialect` — 55 passed (including the new residence/atomicity assertions).
- `cargo test -p smelt-runtime --test statement_parity` — 41 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 104 passed (DuckDB end-state
  unchanged).
- `cargo test -p smelt-backend-trino --test staged_relation_lifecycle` (live tier,
  `trinodb/trino:483` + `apache/iceberg-rest-fixture:1.10.1`) — 1 passed.
- `bash .claude/scripts/large-file-check.sh` — OK (baseline hand-updated with sign-off note).
- `cargo test -p smelt-core --test trino_docs_freshness` — 6 passed.
