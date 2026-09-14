# Phase 11 plan — Surface and close: the Trino state posture reaches the user docs, gated

## Objective

Close the outcome's user-facing half. `smelt explain`'s Trino downgrade rendering already exists
and is tested (phases 4–5, text and `--json`); what is missing is the **published** half — the
docs-site state reference says nothing about which backends realise the correctness structures,
so a Trino user cannot learn what Trino costs or why. This phase publishes that, binds it to
`docs/specs/state.md`'s realisability table with a parsing doc-sync gate so it cannot drift, adds
the two state diagnostics to the docs-site catalogue, and settles the `examples/broken/` clause of
criterion 11 by establishing whether this outcome introduced any new `DiagnosticCode` at all.
Advances criteria 4 (rendered and discoverable), 5 (the refusal is documented) and 11 (gates green,
no baseline bumped).

## Spec delta

None. `docs/specs/state.md` §"Which dialects realise which structure" already carries the Trino
column (phase 2) and `docs/specs/multi_backend.md` already states it for the target. This phase
publishes what those specs already say; it changes no behaviour. If the documentation work
uncovers a spec statement that is wrong, fix the spec in the same commit and say so in the summary.

## Tests

Red-green; all offline, no live coordinator.

1. `crates/smelt-cli/tests/state_docs_freshness.rs::docs_site_names_every_unrealisable_trino_structure`
   — parses the Trino column of `docs/specs/state.md`'s realisability table (same
   parse-don't-restate technique as `trino_docs_freshness.rs::trino_measured_false_capabilities`)
   and asserts `docs-site/docs/reference/state.md` names every structure whose Trino cell is
   `**no**`. A structure added to the spec table without reaching the user docs fails.
2. `…::docs_site_states_the_trino_reason_and_its_permanence` — the docs-site state page states
   the measured reason (autocommit-only Iceberg writes, per-table commit, no cross-table
   transaction), quotes the connector's verbatim answer
   (`Catalog only supports writes using autocommit: iceberg`), and says the absence is permanent
   rather than "not yet" — the distinction the spec draws and the one a user acts on.
3. `…::docs_site_states_what_the_absence_costs_and_what_replaces_it` — the page names the
   recompute-family downgrade, the `MaintenanceStateDowngraded` record, `smelt explain` as where
   to see it, and `DeclaredContractRequiresState` as the one place an absence refuses instead.
4. `…::spark_column_is_not_silently_narrower_than_trinos` — both `Spark (Delta)` and
   `Trino (Iceberg)` spec columns are covered by the same docs-site statement (the page speaks of
   backends with per-table-only atomicity, not of Trino alone), so tightening Spark's column later
   does not leave the user docs describing Trino as a special case. Guards against the
   out-of-scope item "tightening Spark's column is not this outcome's business".
5. `crates/smelt-cli/tests/state_docs_freshness.rs::docs_site_diagnostics_page_lists_the_state_codes`
   — `docs-site/docs/reference/diagnostics.md` carries a row for `MaintenanceStateDowngraded` and
   one for `DeclaredContractRequiresState`, with severities matching `docs/specs/diagnostics.md`'s
   rows (parsed from the spec, not restated).

## Tasks

1. Confirm the criterion-11 fixture clause: `git diff fe6154346^ HEAD -- crates/smelt-db/src/diagnostics_types/`
   over the outcome's commits to establish whether any **new** `DiagnosticCode` variant landed.
   Expectation from the diff survey: none did — every code this outcome uses
   (`MaintenanceStateDowngraded`, `DeclaredContractRequiresState`) pre-existed. Record the finding
   in the summary. If a new variant *did* land, add its `examples/broken/` fixture and
   `docs/specs/diagnostics.md` entry in this phase.
2. Write the five tests above (red) in a new `crates/smelt-cli/tests/state_docs_freshness.rs`.
3. Add a `## Which backends realise these structures` section to
   `docs-site/docs/reference/state.md`, immediately after §"The reconciliation ledger": the
   realisability table restated for users, the measured reason with the connector's verbatim
   answer, permanence, and the downgrade-not-refusal contract with the one named exception.
4. Add the two rows to `docs-site/docs/reference/diagnostics.md` with severities matching the spec.
5. Add a one-paragraph pointer in `docs-site/docs/guide/targets.md`'s `### Trino` section from the
   target to the new state section — the place a user reading about the target asks the question.
6. Cross-check `docs-site/docs/reference/smelt-explain.md`'s existing downgrade paragraph names
   Trino's case (it currently says "no ledger builder yet", which reads as *pending* and is wrong
   for Trino); amend to distinguish pending from permanent.
7. Run the gates; fix anything that falls out without bumping a baseline.

## Verification

- `bash .claude/scripts/verify-phase.sh` — fmt, clippy (both feature sets), shellcheck, full
  `cargo test`, `example_diagnostics`.
- `cargo test -p smelt-cli --test state_docs_freshness` — the new gate.
- `cargo test -p smelt-core --test trino_docs_freshness` — the sibling docs gate stays green.
- `cargo test -p smelt-cli --test trino_explain_downgrade` — the text/`--json` rendering this
  phase documents is unchanged.
- `bash .claude/scripts/large-file-check.sh` — no baseline change.
- `git diff --stat -- .claude/` must be empty: criterion 11 forbids bumping
  `hardening-baseline.txt` or `large-file-baseline.txt`.

## Commit message

`docs(state): publish Trino's state posture — what the absence costs, gated against the spec table`
