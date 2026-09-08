# Phase 11 plan — Close-out: verify every success criterion at HEAD

## Objective

This phase ships no new feature behaviour. It audits the outcome's eight success
criteria against the code and tests that exist at HEAD, runs every gate the criteria
name, and confirms the ratchets are unmoved — so the outcome can be marked `done` on
evidence rather than on the accumulated summaries' say-so. Any criterion whose evidence
turns out to be missing or thinner than claimed is closed here with a test (red-green),
not waved through.

## Spec delta

None. No user-visible behaviour changes in this phase. If the audit finds spec prose
that has drifted from shipped behaviour (e.g. a `docs/specs/sources.md` or
`docs/specs/incremental_models.md` §"The equivalence invariant" paragraph that no longer
matches the retention code), correcting that prose is in scope and comes before anything
else — but no new normative rule is introduced.

## Tests

The audit is the work; these are the assertions it must be able to point at, verified to
exist and to pass at HEAD. Write a test only where the audit finds one absent.

1. **Criterion 1 (quantifier)** — `docs/specs/incremental_models.md` states the retained
   -vs-all-history quantifier with reasoning; the conformance oracle
   (`maintenance_conformance` `s_restricted_oracle_sql` in `s_tracker.rs`) still
   materialises from tracker-recorded rows, not the physical source relation. Read both;
   no new test unless they disagree.
2. **Criterion 2 (declaration)** — the four `examples/broken/models/sources/retention_*.yml`
   fixtures each refuse with a named code; `cargo test -p smelt-cli --test example_diagnostics`
   (`retention_diagnostics.rs`) and `diagnostics_catalogue` green.
3. **Criterion 3 (walk)** — `cargo test -p smelt-logical --test walk_coverage` green and the
   retention verdict is produced inside `analysis/walk.rs`; confirm no ad hoc scan was
   reintroduced (`rg 'retention' crates/smelt-logical/src` — every production site either
   inside the walk or a walk-invoked leaf classifier with the doc-comment classification).
4. **Criterion 4 (refuse or degrade, never silent)** —
   `smelt-logical`'s `every_retention_verdict_maps_to_a_refusal_a_downgrade_or_an_admitted_fit`
   plus the three sibling cases in `crates/smelt-logical/tests/retention_admission.rs`.
5. **Criterion 5 (the bound moving is an event)** — the bound-advance test in
   `crates/smelt-runtime/tests/retention_admission.rs`; assert it is run-time (current
   bound) rather than authoring-time.
6. **Criterion 6 (conformance)** — `crates/smelt-cli/tests/maintenance_conformance/gate/retention_pool.rs`
   exists as a `SourceRecipe` pool whose bound advances between run steps; the seeded
   sample passes.
7. **Criterion 7 (explain + docs)** — `crates/smelt-cli/tests/explain_model/retention.rs`
   (text + JSON) and the two-sided `docs_site_diagnostics_reference_lists_every_source_retention_code`
   sync test; `cli_docs_coverage` green.
8. **Criterion 8 (gates + ratchets)** — the Verification list below, plus every baseline in
   `.claude/` unchanged in `git status` after the run.

## Tasks

1. Re-read the outcome's Success criteria; build an evidence table (criterion → file:test →
   pass/fail) as you go. Do not consult the phase summaries for evidence — read the code.
2. Criterion 1: read `docs/specs/incremental_models.md` §"The equivalence invariant" and
   `docs/specs/sources.md`; confirm the quantifier paragraph and its reasoning are present and
   match `s_tracker.rs`'s oracle. Fix drifted prose if found.
3. Criteria 2-7: for each, locate the named evidence and run its gate; record the exact test
   name and the pass line.
4. For any criterion whose evidence is absent or does not actually assert what the criterion
   claims, write the missing test red-green now (this is the only production/test-authoring
   work the phase may do); if closing it would need new production behaviour, stop and flip
   the outcome to `blocked` with the decision a human must make.
5. Run the full Verification list below, in the foreground, and capture the tails.
6. Confirm ratchets unmoved: `git status --porcelain .claude/` clean, and
   `bash .claude/scripts/large-file-check.sh` OK.
7. Write `phases/11-summary.md` carrying the evidence table verbatim — it is the artifact the
   outcome's `done` ruling rests on.
8. Append a dated evidence line to the outcome's Decision log, flip row 11 to `done`.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test walk_coverage --test retention_admission`
- `cargo test -p smelt-runtime --test retention_admission --test retention_full_refresh --test statement_parity --test execute_parity`
- `cargo test -p smelt-cli --test maintenance_conformance --test example_diagnostics --test cli_docs_coverage`
- `cargo test -p smelt-cli --test explain_model --test explain_maintenance`
- `bash .claude/scripts/large-file-check.sh` and `git status --porcelain .claude/`

## Commit message

`outcome(trimmed-history-sources): close out with a verified criterion-to-evidence audit`
