# Phase 3d plan — Phase 3's deferred live legs: the two ledger-free families through `execute_project`, and `statement_parity`'s Trino leg

## Objective

Land the two live legs phase 3 deferred when gaps 1–3 blocked them (all three now closed by 3a/3b2/3c):
the insert-only **append** family and the whole-row **`MERGE` upsert** (keyed-fold) family driven
end-to-end through a real `execute_project` run against the live Trino tier, each asserted equal to a
full-refresh oracle rather than to pinned rows; plus `statement_parity`'s Trino **executed-vs-emitted**
leg over the same keyed-fold run. Advances criteria 2 and 5 (executed half), and puts the new legs in
front of CI so they cannot pass vacuously.

## Spec delta

None. This phase proves behaviour the spec already states (`multi_backend.md` §"Incremental & schema
evolution per backend"'s Trino rows, landed in phase 2); it changes no user-visible surface. If a leg
measures something the landing table does not claim, that is an escalation, not a silent spec edit.

## Tests (red first)

1. `crates/smelt-cli/tests/trino_incremental_families.rs::append_family_matches_full_refresh_on_trino`
   (live) — a `grain: partition` insert-only model over an append-only source, run as two disjoint
   windowed runs, ends multiset-equal to a `--full-refresh` rebuild of the same models into a second
   schema (oracle), not to a pinned row list.
2. `…::whole_row_merge_upsert_matches_full_refresh_on_trino` (live) — a `refresh: incremental`,
   `grain: key`, `unique_key:` model with a fold-eligible combiner (`SUM`) over a **clocked**
   append-only source, so its cell resolves to the keyed-fold whole-row `MERGE` (not the 3c
   downgrade). Run over two disjoint windows where the second both updates an existing key's total
   (matched arm) and introduces a new key (not-matched arm); ends multiset-equal to the full-refresh
   oracle. Red today only if the family does not in fact run — record the measured error if so.
3. `crates/smelt-runtime/tests/statement_parity/trino.rs::keyed_fold_parity_on_trino` (live,
   `SMELT_TRINO_URL`-gated, prints a `Skipping …` line when unset) — a `RecordingBackend` over a real
   `TrinoBackend` captures the `StatementGroup`s a real `execute_project` run sends for test 2's
   model, asserted byte-identical to a direct `emit_keyed_fold` call with the batch's own inputs
   (shape: `structural_and_ledger.rs::snapshot_reconcile_delete_leg_parity`), and the post-run table
   is `multiset_equal` to a full refresh.
4. `crates/smelt-cli/tests/trino_ci_wiring.rs::the_trino_job_runs_every_live_gated_trino_test_binary`
   — the live-gated census is **derived** (scan `crates/smelt-{cli,backend-trino,runtime}/tests` for
   files naming `SMELT_TRINO_URL` or `trino_env(`), not a hardcoded list, and every derived binary
   must appear in a `trino-integration` step; fails today because `trino_incremental_families`,
   `trino_state_residency`, `trino_ddl_live`, … and the new `statement_parity` leg are unrun in CI.
5. `…::every_trino_gated_test_file_skips_through_the_shared_env_gate` — same derived census replaces
   the stale hardcoded `live_gated_files` array; existing per-file assertions unchanged.

## Tasks

1. Generalize `RecordingBackend`/`RecordingBackendFactory` (`crates/smelt-runtime/tests/statement_parity/main.rs`)
   to hold `inner: Box<dyn Backend>` so the same recorder wraps DuckDB or Trino; keep every existing
   DuckDB leg passing unchanged.
2. Add `smelt-backend-trino` to `smelt-runtime`'s `[dev-dependencies]` with a one-line comment stating
   it is test-only (same shape as the existing `smelt-cli` back-edge comment).
3. Add `mod trino;` + `crates/smelt-runtime/tests/statement_parity/trino.rs` with a local
   `SMELT_TRINO_URL` gate that returns `Option` and prints `Skipping …` (no `unwrap_or` default on the
   lookup — wiring test 5 checks this), plus a unique-schema helper (`format!("smelt_sp_{}", process::id())`)
   and an always-fires schema drop.
4. Write test 3's leg: stage the keyed-fold project, run `execute_project` through the recording
   factory, diff recorded groups against `emit_keyed_fold`, then `multiset_equal` against a full refresh.
5. Write tests 1 and 2 in `trino_incremental_families.rs`, each staging its own `trino_schema(label)`
   and dropping it at the end; the oracle arm rebuilds into a second schema with `--full-refresh` and
   compares sorted `fetch_trino_rows` output.
6. Rewrite `trino_ci_wiring.rs`'s census as a directory scan (one helper, used by both tests 4 and 5),
   and add the `trino-integration` steps that make it pass: one running every live-gated `smelt-cli`
   Trino binary, one running `cargo test -p smelt-runtime --test statement_parity`, both under
   `set -o pipefail` with the existing `grep -qi skipping` no-skip guard and the `always()` teardown.
7. Update `trino_incremental_families.rs`'s module doc: the three gaps are closed, and this file now
   carries the two family legs phase 3 deferred.
8. Write `phases/03d-summary.md`.

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt, clippy both feature sets, shellcheck, full `cargo test`,
  `example_diagnostics`).
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity` (no live tier: the Trino
  leg must skip cleanly and every DuckDB leg must still pass after task 1's refactor).
- `cargo test -p smelt-cli --test trino_ci_wiring`.
- Live tier (`bash scripts/trino-up.sh`; `source scripts/trino-env.sh`; `bash scripts/trino-down.sh`),
  run serially per 3b2's interim workaround until phase 3e lands isolation:
  `cargo test -p smelt-cli --test trino_incremental_families -- --test-threads=1` and
  `cargo test -p smelt-runtime --test statement_parity -- --test-threads=1`.
- The live tier is required: if the coordinator is unreachable, emit `<<PHASE_BLOCKED>>` rather than
  accepting the skip path as green.

## Commit message

`feat(trino): prove the append and keyed-fold MERGE families end-to-end through execute_project, with statement_parity's Trino leg`
