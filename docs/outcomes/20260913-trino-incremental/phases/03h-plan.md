# Phase 3h plan — the whole-row `MERGE` upsert family live on Trino, and `statement_parity`'s Trino executed-vs-emitted leg

## Objective

Re-attempt the two legs phase 3d measured as blocked, now that 3f closed gap 4 (the windowed-keyed
driver's partition literals go through the single typed renderer) and 3g closed gap 5 (`KeyedFold`
resolves its state requirement by grade at plan time). Drive the whole-row `MERGE` upsert family
end-to-end through a real `execute_project` run against the live Trino coordinator, asserted equal to
a full-refresh oracle, and prove the statements it executes are byte-identical to what
`smelt-logical`'s emitter produced. Advances criteria 2 and 5 (executed half), plus criteria 3 and 8
for the additive grade's downgraded route.

**Grade matters, and 3d's design predates 3g.** 3d's test 2 used a `SUM` combiner; under 3g's ruling
`SUM` is `Grade::Additive` and therefore *downgrades to the whole-target rebuild* on a
structure-less backend — that is the never-fold-twice route, not the `MERGE` family. So the family
criterion 2 names is reached with an **idempotent** combiner (`MIN`/`MAX`, the `events_deduped`
shape), and the additive fixture becomes the criterion-3/8 downgrade proof, run live in the same
phase because 3g proved that route only on DuckDB.

## Spec delta

None. This phase proves behaviour the spec already states (`multi_backend.md` §"Incremental & schema
evolution per backend"'s Trino rows and `state.md` §"The degradation contract" step 2, landed in
phases 2 and 3g). If a leg measures something those sections do not claim, that is an escalation to
the decision log, not a silent spec edit.

## Tests (red first)

1. `crates/smelt-cli/tests/trino_incremental_families.rs::whole_row_merge_upsert_matches_full_refresh_on_trino`
   (live) — a `refresh: incremental`, `grain: key`, `MIN`-combiner (idempotent) model over a clocked
   append-only source with a typed partition column, run over two disjoint windows where the second
   both revises an existing key (matched arm) and introduces a new key (not-matched arm); the
   maintained table ends multiset-equal to a `--full-refresh` rebuild into a second schema.
2. `…::whole_row_merge_upsert_writes_through_the_merge_route_on_trino` (live) — the same run's
   `--explain`/stdout shows the cell resolving to the keyed-fold `MERGE` route and **not**
   downgraded, so test 1 cannot pass vacuously via the rebuild path.
3. `…::additive_keyed_fold_downgrades_and_still_matches_full_refresh_on_trino` (live) — the same
   fixture with a `SUM` combiner: the cell is recorded downgraded (explain-visible, naming the
   missing structure) and the maintained table is *still* multiset-equal to the full-refresh oracle
   after each of two windows — criterion 8's "a downgraded cell is asserted oracle-equal, not
   exempted", on the live tier.
4. `crates/smelt-runtime/tests/statement_parity/trino.rs::keyed_fold_parity_on_trino` (live,
   `SMELT_TRINO_URL`-gated with an explicit `Skipping …` line, no `unwrap_or` default) — a
   `RecordingBackend` wrapping a real `TrinoBackend` captures the `StatementGroup`s a real
   `execute_project` run sends for test 1's model, asserted byte-identical to a direct
   `smelt_logical::maintenance::emit::emit_keyed_fold` call with the batch's own inputs (shape:
   `structural_and_ledger.rs::snapshot_reconcile_delete_leg_parity`).
5. `crates/smelt-cli/tests/trino_ci_wiring.rs` (existing derived census) — passes unchanged with the
   new `smelt-runtime` `statement_parity` Trino binary present in the `trino-integration` job; extend
   `discover_test_binaries`' scan root to `crates/smelt-runtime/tests` so the new leg is actually
   censused rather than invisible to it.

## Tasks

1. Generalize `RecordingBackend`/`RecordingBackendFactory` (`crates/smelt-runtime/tests/statement_parity/main.rs`)
   to hold `inner: Box<dyn Backend>` so one recorder wraps DuckDB or Trino; every existing DuckDB leg
   must pass unchanged.
2. Add `smelt-backend-trino` to `smelt-runtime`'s `[dev-dependencies]` with a one-line test-only
   comment (same shape as the existing `smelt-cli` back-edge comment).
3. Add `mod trino;` + `crates/smelt-runtime/tests/statement_parity/trino.rs`: local env gate returning
   `Option`, a process-unique schema per 3e's isolation rule, an always-fires schema drop, and test 4.
4. Add a `stage_keyed_fold_project(tmp, schema, combiner)` helper to `trino_incremental_families.rs`
   (one helper, `MIN` vs `SUM` parameterised) staging a clocked source + keyed mart, and write tests
   1–3 against it, each with its own `trino_schema(label)` and oracle schema, dropped at the end.
5. Extend `trino_ci_wiring.rs`'s census scan root to `crates/smelt-runtime/tests` and add the
   `trino-integration` step in `.github/workflows/compat.yml` running
   `cargo test -p smelt-runtime --test statement_parity`, under `set -o pipefail` with the existing
   `grep -qi skipping` no-skip guard and `always()` teardown.
6. Update `trino_incremental_families.rs`'s module doc: gaps 4 and 5 are closed (3f/3g), this file now
   carries the `MERGE` family leg and the additive downgrade's live oracle proof.
7. Write `phases/03h-summary.md`, recording whether Trino's idempotent arm's
   `realises_merge_ledger(dialect)`-only bookkeeping guard (3g's untouched watch item) surfaced.

## Verification

- `bash .claude/scripts/verify-phase.sh`.
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity` (no live tier: the Trino
  leg must skip cleanly and every DuckDB leg must still pass after task 1's refactor).
- `cargo test -p smelt-cli --test trino_ci_wiring`.
- Live tier (`bash scripts/trino-up.sh`; `source scripts/trino-env.sh`; `bash scripts/trino-down.sh`):
  `cargo test -p smelt-cli --test trino_incremental_families` and
  `cargo test -p smelt-runtime --test statement_parity`.
- The live tier is **required**: if the coordinator is unreachable, emit `<<PHASE_BLOCKED>>` rather
  than accepting the skip path as green.

## Commit message

`feat(trino): prove the whole-row MERGE upsert family end-to-end on Trino, with statement_parity's Trino leg`
