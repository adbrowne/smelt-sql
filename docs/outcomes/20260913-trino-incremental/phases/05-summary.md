# Phase 5 summary — the merge-less conditional write on Trino, and the column-scoped merge's downgrade

**Shipped:**
- Measured live (2026-09-15): `DELETE FROM t USING s WHERE ...` refuses on Trino with `mismatched
  input 'USING'. Expecting: '.', '@', 'WHERE', <EOF>`; the correlated `WHERE EXISTS` form is
  accepted and computes the same rows. Recorded in `docs/specs/multi_backend.md` §"Column-scoped
  merge and conditional-write capabilities".
- `smelt_logical::maintenance::emit::staged::changed_row_delete` — the single shared helper
  rendering the changed-row `DELETE` for a dialect (`USING` on DuckDB/Spark/BigQuery, correlated
  `WHERE EXISTS` on Trino). Routed all four production `DELETE ... USING` sites through it:
  `emit_staged_candidate_conditional`, `emit_staged_candidate_conditional_recompute`
  (`crates/smelt-logical/src/maintenance/emit/staged.rs`), `emit_per_group_recompute` and
  `emit_diff_patch`'s update leg (`crates/smelt-logical/src/maintenance/emit/recompute.rs`).
- `StagedRelation::derive_for_capabilities` (`crates/smelt-logical/src/maintenance/emit/staged_relation.rs`)
  — reads `staged_relation_residence`/`staged_relation_group_is_atomic` off `BackendCapabilities`.
  Threaded into all six production derivation sites that used to hardcode
  `StagedRelationResidence::SessionTemporary`/`atomic: true`: `membership/execute.rs` (recompute +
  keyless staged/sentinel pair), `delta_restriction/mod.rs`'s `build_delete_insert_group_dispatched`
  (now takes `capabilities: &BackendCapabilities`, threaded from both the live executor and
  `dry_run.rs` via a new `execute::targets::capabilities_for_target`), `repair/execute.rs`'s
  `repair_staged_relation`/`diff_patch_staged_relation` (now take a capabilities argument),
  `diagnostics/preview.rs`'s technique preview, and `cumulative.rs`'s `write_group` override (the
  `WindowedKeyedRule` trait method itself grew a `capabilities: &BackendCapabilities` parameter).
- `SqlCompiler::capabilities()` accessor (`crates/smelt-runtime/src/compile.rs`) and
  `execute::targets::capabilities_for_target` — the no-live-backend capability lookups a preview or
  dry-run derivation site needs, sharing `compile.rs`'s existing `BackendType -> (SqlDialect,
  BackendCapabilities)` match rather than restating it.
- Live proofs (`crates/smelt-backend-trino/tests/staged_group_live.rs`): the `USING`-vs-`EXISTS`
  measurement, and a real `TargetSchema`, non-atomic staged-candidate conditional group executed
  end to end through `execute_statement_group`.
- `crates/smelt-cli/tests/trino_incremental_families/{membership_and_merge.rs,...}`: test 8
  (a membership-sensitive model — clocked fact join a `mutable_snapshot` dimension — oracle-equal to
  `--full-refresh` after a genuine departure) and test 9 (a `ColumnScopedMerge`-electing model
  downgrades on Trino with a named `missing` reason, still oracle-equal after the dimension
  mutates).
- `crates/smelt-runtime/tests/statement_parity/trino.rs::staged_candidate_conditional_parity_on_trino`
  — the executed membership-recompute group is byte-identical to a direct
  `emit_staged_candidate_conditional_recompute` call.
- Two new structural tests in `crates/smelt-runtime/tests/staged_relation_atomicity.rs`:
  no production line under `crates/smelt-runtime/src/` spells the hardcoded shape, and every
  derivation site (including the `repair_staged_relation`/`diff_patch_staged_relation` wrappers)
  yields `TargetSchema`/non-atomic under Trino's capabilities and `SessionTemporary`/atomic under
  DuckDB's.

**Decisions:**
- The generic `USING`→`EXISTS` transformation preserves semantics uniformly across all four sites,
  including `emit_diff_patch`'s update leg (whose predicate carries a `slice_predicate` clause that
  does not reference the staged relation) — wrapping the WHOLE predicate inside the correlated
  subquery is equivalent because a clause independent of the subquery's own table evaluates
  identically inside or outside it. One helper, no per-site special-casing.
- `capabilities_for_target`/`SqlCompiler::capabilities()` reuse `compile.rs`'s existing
  `dialect_and_capabilities` match (made `pub(crate)`) rather than a second `BackendType` match —
  single owner for that mapping.
- `WindowedKeyedRule::write_group` grew a `capabilities` parameter on the trait itself (not just the
  override) even though the default `Merge` arm ignores it, because the trait's own doc contract
  ("an override's `StagedCandidate` arm must derive its `StagedRelation` from it") is the correct
  place to state the obligation.
- Registered a small, direct baseline bump for `compile.rs` (+10 lines) and `cumulative.rs`
  (+4 lines) via `.claude/scripts/large-file-check.sh --update` — both are the minimal cost of
  threading `BackendCapabilities` through an existing accessor/trait method, not unrelated growth.
  `crates/smelt-cli/tests/trino_incremental_families.rs` crossed the 1500-line cap after tests 8-9
  landed; split into a `trino_incremental_families/` directory target (`main.rs` +
  `keyed_fold.rs`/`delete_insert.rs`/`membership_and_merge.rs` submodules) instead of registering an
  oversized-file exception — all 12 tests (the 10 pre-existing plus the 2 new ones) still pass
  unchanged under the split.

**For the next planner:**
- Phase 6 is next: the degraded routes (per-group recompute, succession's full rebuild, the
  sidecar-less key-addressed downgrade) plus succession's own partition-literal call site in
  `execute/project/mod.rs` (still passes `Undeclared` — 3f's untouched residue) and
  `statement_parity`'s Trino byte-identity leg for the additive keyed fold's downgrade route.
- Not done in this phase (out of scope per the plan): phases 7-10 (structural no-authoring leg,
  the generative `maintenance_conformance` Trino leg, the contract lattice, mid-stream schema
  evolution) remain `pending`.
- No new gaps discovered — the two measured facts (the `USING` refusal text, and that the
  correlated `EXISTS` form both parses and computes correctly) matched what the spec delta
  anticipated before the live probe ran.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full workspace `cargo test`, example_diagnostics).
- `cargo test -p smelt-logical --test walk_coverage` — 14 passed.
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity --test staged_relation_atomicity` — 4 + 5 + 44 passed (offline; Trino leg skips green without `SMELT_TRINO_URL`, separately proven live below).
- `cargo test -p smelt-cli --test trino_incremental_spec_freshness --test trino_ci_wiring --test state_docs_freshness` — 6 + 8 + 9 passed.
- Live tier (`bash scripts/trino-up.sh` / `source scripts/trino-env.sh`, torn down with `bash scripts/trino-down.sh` at the end):
  - `cargo test -p smelt-backend-trino --test staged_group_live -- --test-threads=1` — 2 passed.
  - `cargo test -p smelt-cli --test trino_incremental_families -- --test-threads=1` — 12 passed.
  - `cargo test -p smelt-runtime --test statement_parity -- --test-threads=1` — 44 passed (including the new `staged_candidate_conditional_parity_on_trino`).
