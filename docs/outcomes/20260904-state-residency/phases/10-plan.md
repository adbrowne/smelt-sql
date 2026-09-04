# Phase 10 plan — close the keyed-grain residue outcome; refresh the docs-site state pages

## Objective

Discharge success criterion 7 (amend `docs/outcomes/20260815-keyed-grain-residue`'s criterion 3
to the decided downgrade wording, close its blocked phase 3 against this outcome's criterion 3,
and mark that outcome `done`) and the docs-site half of criterion 8 (every user-facing page that
still describes `.smelt/reconciliation.json`, an unconditional `.smelt/` write set, or a
fail-loud ledger-less refusal is rewritten to what phases 2-9 actually landed). Criterion 5's
user-visible surface — `state.warehouse_tables` — is documented here too: `docs-site`'s
`smelt.yml` reference has no `state` key at all today.

## Spec delta

None. `docs/specs/state.md`, `run_state.md`, `incremental_models.md`, `incremental_shapes.md`,
`smelt_yml.md` and `diagnostics.md` were all brought to the landed behaviour by phases 1, 4, 7
and 9. This phase propagates those already-normative statements into `docs-site/` and into the
sibling outcome's record. (`state.md` §Known Divergences is phase 11's work, not this phase's.)

## Tests

New gate file `crates/smelt-cli/tests/state_docs_freshness.rs` — three standing user-docs checks,
each red before the doc edits:

1. `user_docs_never_claim_a_reconciliation_json_file` — no file under `docs-site/docs/` contains
   the string `reconciliation.json`; the ledger is engine-resident (`_smelt_ledger`) since phase 2.
2. `smelt_yml_reference_documents_the_state_block` — `docs-site/docs/reference/smelt-yml.md`
   contains a `state` top-level-field row and documents both `mode` and `warehouse_tables`
   (`allowed`/`none`), matching `docs/specs/smelt_yml.md`'s row.
3. `state_reference_states_the_residency_invariant` — `docs-site/docs/reference/state.md`
   contains a per-posture write-set section naming `stateless`, and its recovery playbook states
   that deleting `.smelt/` does not change what a maintained model computes.

## Tasks

1. Write `state_docs_freshness.rs` with the three tests; confirm all three fail (red).
2. `docs-site/docs/reference/state.md`: delete the `reconciliation.json` inventory row; fix the
   lazy-creation sentence (§Inventory) and the locking sentence's shared-file list.
3. `docs-site/docs/reference/state.md` §"The reconciliation ledger": state engine residency —
   a per-model `_smelt_ledger` table in the target backend, folded in the same transaction as the
   write it protects — and replace the "on a backend with no ledger substrate … says so on the
   run's progress output" sentence with the recorded downgrade to the recompute-family equivalent,
   visible in `smelt explain` and as the `MaintenanceStateDowngraded` warning.
4. `docs-site/docs/reference/state.md`: add a §"`state.mode` and what is written" section giving
   the per-posture write set landed in phase 8 (`stateless` writes nothing under `.smelt/`;
   `--resume` refuses under it by naming the posture), sourced from `docs/specs/state.md`
   §"`state.mode` and what each posture provides" — do not restate the spec's rationale.
5. `docs-site/docs/reference/state.md` §"Recovery playbook": lead the "`.smelt/` is lost" entry
   with the invariant — deleting `.smelt/` never changes what a maintained model computes; what is
   lost is skip-coverage bookkeeping, not correctness state.
6. `docs-site/docs/guide/deployment.md`: drop `reconciliation.json` from the layout tree and from
   the sentence below it.
7. `docs-site/docs/reference/cli.md` (§State isolation per target) and
   `docs-site/docs/reference/smelt-yml.md` (§Targets, "State isolation per target"): drop
   "reconciliation ledgers" from the per-target `.smelt/` artifact lists — a per-target file store
   no longer holds one.
8. `docs-site/docs/reference/smelt-yml.md`: add the `state` top-level-field row and a
   §"State Configuration" section covering `mode` (`stateless` | `intervals` | `environments`) and
   `warehouse_tables` (`allowed` | `none`), naming `MaintenanceStateDowngraded` and
   `DeclaredContractRequiresState` as `none`'s only two consequences.
9. `docs-site/docs/guide/targets.md`: rewrite the Spark `Additive`-combiner coverage row — the
   cell takes a recorded, explain-visible downgrade to its recompute-family equivalent, not a
   fail-loud refusal.
10. `docs-site/docs/reference/smelt-explain.md` (line 27 list) and
    `docs-site/docs/guide/incremental-models.md` §"The reconciliation ledger": add/repair the
    residency statement and note that `smelt explain` prints a cell's recorded state downgrade
    (text and `--json`).
11. `docs/outcomes/20260815-keyed-grain-residue/outcome.md`: amend criterion 3 to "on a backend
    with no ledger realisation the cell takes a recorded, explain-visible downgrade to its
    recompute-family equivalent (`state.md` §'The degradation contract'); the fold is transactional
    wherever it happens"; flip the phase 3 row `blocked` → `done`; append a dated Decision-log
    entry recording that option 1 was chosen on 2026-09-04 and discharged by
    `docs/outcomes/20260904-state-residency` phases 4-7; append a dated resolution line under
    §Blocked pointing at the same; set `**Status:**` to `done`.
12. Re-run the three new tests (green) plus the doc-sync/tutorial gates.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test state_docs_freshness`
- `cargo test -p smelt-cli --test tutorial_freshness --test cli_docs_coverage --test rebuild_dry_run`
- `rg -n "reconciliation\.json" docs-site/ docs/specs/` — no hits outside `docs/plans/`,
  `docs/outcomes/`, `docs/handoffs/`.

## Commit message

`docs(state-residency): refresh docs-site state pages and close the keyed-grain residue outcome`
