# Phase 1 summary — Characterise Iceberg `MERGE` by execution

## Shipped

- `crates/smelt-backend-trino/tests/merge_clause_forms.rs` (9 tests): measures every `MERGE` clause
  form phase 3/5's emitters need, against the live tier — acceptance and, for accepted forms, a
  value leg (guard selectivity, first-match-wins ordering, table contents after apply).
- `crates/smelt-backend-trino/tests/common/mod.rs`: the `LiveEnv` harness lifted out of
  `capability_probes.rs` so `merge_clause_forms.rs` doesn't duplicate it — schema lifecycle, `ok`,
  plus new `err_text`/`select_i64_col`/`select_string_col` for this phase's value legs.
  `capability_probes.rs` now uses the shared module; its 25 tests still pass unchanged.
- `docs/outcomes/20260913-trino-incremental/outcome.md` §Decision log: the dated measurement table
  with every clause form's verdict and quoted error text (criterion 1's "measured errors quoted").
- `crates/smelt-cli/tests/trino_ci_wiring.rs`: added `merge_clause_forms.rs` to the live-gated file
  list; taught the "must print a skipping line" check to also read `tests/common/mod.rs` for files
  using `mod common;`, since the skip print now lives there, not duplicated per file.

## Decisions

- **No `BackendCapabilities`/spec matrix change.** `supports_merge` and `supports_column_scoped_merge`
  measured `true` as already recorded; `supports_merge_not_matched_by_source` measured `false`,
  confirming `capability_probes.rs`'s existing probe. Expected outcome per the plan, not a surprise.
- **Emitter target fixed for phases 3 and 5: named-column form only.** `UPDATE SET *`, `INSERT *`,
  and `INSERT ROW` are all refused by Trino's grammar (mismatched-input parse errors, not a
  semantic rejection) — every `SET`/`INSERT` must enumerate columns. This is the one finding later
  phases must act on, not merely note.
- **Harness lifted to `tests/common/mod.rs`** per the plan's explicit instruction, rather than
  copy-pasting a third `LiveEnv` into `merge_clause_forms.rs`. `staged_relation_lifecycle.rs` keeps
  its own independent (older, slightly different) copy — out of this phase's scope, left alone.

## For the next planner

- Phase 3 ("append and whole-row-`MERGE` upsert families... including landing `maintenance_dialect`
  for `SqlDialect::Trino`") should render `MERGE`'s `UPDATE SET`/`INSERT` clauses column-by-column
  from the model's own schema — there is no star shorthand to fall back to on this backend.
- The `WHEN MATCHED THEN DELETE` arm is confirmed available; phase 5/6 can use it for the
  merge-less conditional-write and delete-and-insert routes without a separate probe.
- `MERGE`'s `USING` clause accepts a bare subquery over a real table (the staged-relation shape);
  no further probing needed there before phase 3 starts building statements.
- Did not touch: `docs/specs/multi_backend.md` prose (phase 2's job — no matrix cell changed, so
  phase 2 has no correction to make, only the narrative to write) or any statement-emission code.

## Gates

- `source scripts/trino-env.sh && cargo test -p smelt-backend-trino --test merge_clause_forms` — 9
  passed, 0 skipped.
- `cargo test -p smelt-backend-trino --quiet` (live tier up) — 82 tests passed, 0 skipped
  (`grep -i skip` empty).
- `cargo test -p smelt-core --test trino_docs_freshness` — 6 passed, unchanged.
- `bash .claude/scripts/large-file-check.sh` — OK.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full workspace `cargo test`, `example_diagnostics`).
- `bash scripts/trino-down.sh` — tier torn down.
