# Phase 11 summary — Surface and close

## Shipped

- `docs-site/docs/reference/state.md` — new `## Which backends realise these structures` section
  (after §"The reconciliation ledger"): the realisability table restated for users, the measured
  autocommit refusal quoted verbatim, permanence stated explicitly, and the downgrade-not-refusal
  contract with the one named exception (`DeclaredContractRequiresState`).
- `docs-site/docs/reference/diagnostics.md` — new `## State residency` section with a `Code |
  Severity | Trigger` table for `MaintenanceStateDowngraded` (Warning) and
  `DeclaredContractRequiresState` (Error), linked from the new state.md section.
- `docs-site/docs/guide/targets.md` — Trino section gains a pointer paragraph from the target to
  the new state-residency section, naming the five unrealisable structures and the permanence.
- `docs-site/docs/reference/smelt-explain.md` — the downgrade paragraph's "no ledger builder yet"
  wording (which reads as *pending*) is corrected to distinguish `state.warehouse_tables: none`
  from a backend realising none of the five structures at all (Spark, Trino — permanent), with a
  pointer to the new state.md section.
- `crates/smelt-cli/tests/state_docs_freshness.rs` — 5 new tests, all parsing the spec rather than
  restating it: `docs_site_names_every_unrealisable_trino_structure`,
  `docs_site_states_the_trino_reason_and_its_permanence`,
  `docs_site_states_what_the_absence_costs_and_what_replaces_it`,
  `spark_column_is_not_silently_narrower_than_trinos`,
  `docs_site_diagnostics_page_lists_the_state_codes`.

## Decisions

- **No new `DiagnosticCode`.** `git diff fe6154346^ HEAD -- crates/smelt-db/src/diagnostics_types/`
  over the outcome's commits shows no new variant — `MaintenanceStateDowngraded` and
  `DeclaredContractRequiresState` both pre-existed. No `examples/broken/` fixture work needed;
  criterion 11's fixture clause is satisfied vacuously.
- **State residency gets its own diagnostics-page section**, mirroring the existing "Source
  retention" / "Succession grain" pattern (own `## ` heading, own `Code | Severity | Trigger`
  table) rather than folding into the generic "Code catalogue" prose list — the two codes are a
  coherent pair (downgrade vs. refusal) worth a shared explanatory paragraph.
- **The Spark/Trino distinction is stated as one shared cause**, not two backend-specific
  writeups, so the new section doesn't silently narrow to read as Trino-only (guarded by
  `spark_column_is_not_silently_narrower_than_trinos`).

## For the next planner

- Nothing deferred out of this phase; all 5 planned tests and the 6 doc-edit tasks landed.
- `docs-site/docs/reference/smelt-explain.md`'s downgrade paragraph now names Spark's realise-none
  posture alongside Trino's — worth a glance if a future Spark-tightening outcome changes Spark's
  column (out of scope for *this* outcome, per its own "Out of scope" list, but the shared-cause
  framing here means that change would need a doc touch too).
- The outcome's own Phases table has no remaining `planned` row after this — `20260913-trino-ledger`
  is ready to flip to `done` at the outcome level once this phase's row is marked `done`.

## Gates

- `cargo test -p smelt-cli --test state_docs_freshness` — 9/9 passed (4 pre-existing + 5 new).
- `cargo test -p smelt-core --test trino_docs_freshness` — 6/6 passed, unchanged.
- `cargo test -p smelt-cli --test trino_explain_downgrade` — 5/5 passed, unchanged.
- `bash .claude/scripts/large-file-check.sh` — OK, no baseline change.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full `cargo test`, `example_diagnostics`).
- `git diff --stat -- .claude/` — empty; no baseline bumped.
