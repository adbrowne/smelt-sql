# Phase 9 plan — Close-out

## Objective

Verify at HEAD that each of the outcome's seven success criteria has real, re-runnable
evidence rather than a summary's claim; confirm the hardening/large-file/parser ratchets sit
where the phase summaries left them; and hand the residue this outcome surfaced back to
`20260906-bigquery-dogfood-spine` through its findings handoff so the spine's blocked
live-BigQuery phases pick it up. This phase writes no feature code — it advances criterion 7
directly and converts criteria 1-6 from asserted to demonstrated.

## Spec delta

None. Phases 1, 4, 5 and 6 landed the whole normative surface (`docs/specs/sources.md`,
`docs/specs/run_state.md`, `docs/specs/cli.md`, `docs/specs/diagnostics.md`); close-out
verifies it rather than changing it. If verification finds a spec claim the code does not
honour, that is a finding for the handoff, not a silent spec edit.

## Tests

No new product tests. The phase's own artifact is a verification record, plus one small
standing gate for the doc gap phase 8 named:

1. `external_step_docs_freshness::explain_reference_mentions_external_steps` — red today:
   `docs-site/docs/reference/cli.md`'s `smelt explain` material never names external steps,
   though `smelt explain <step>` is spec'd in `cli.md` §"smelt explain <external step>".
   Green after the reference page gains the step form. (Phase 8's "for the next planner"
   item; the only real behaviour-vs-docs asymmetry it left.)

Everything else is re-running the existing gates listed under Verification and recording
their output — no new assertions.

## Tasks

1. Re-read the outcome's success criteria; for each of 1-6, identify the *specific* gate,
   test name or committed file that is its evidence, and run it at HEAD. Record the exact
   command and its pass/fail in `phases/09-summary.md` as a criterion → evidence table.
   Criterion-to-evidence starting points (verify, don't assume):
   - 1 spec: `docs/specs/sources.md` §"Externally-produced sources"; the timeless-oracle
     check is `external_step_docs_freshness::no_plan_vocabulary` plus a grep for
     `Phase [A-Z0-9]` in the changed spec sections.
   - 2 declaration/validation: `cargo test -p smelt-db --test diagnostics_catalogue`, the
     `examples/broken/` external-step fixtures, `smelt-core` external_step unit tests.
   - 3 DAG membership: `cargo test -p smelt-cli --test list_external_step`, graph
     `select_nodes`/`steps_required_by` tests.
   - 4 invocation/failure: `cargo test -p smelt-runtime --test external_steps` (or the
     phase-4 test file's real name), including the refusal legs.
   - 5 explain: `cargo test -p smelt-cli --test explain_external_step --test cli_docs_coverage`.
   - 6 fixture + docs: `cargo test -p smelt-cli --test example_diagnostics`,
     `cargo test -p smelt-lsp --test example_workspaces`,
     `cargo test -p smelt-cli --test external_step_docs_freshness`,
     `cd docs-site && uv run mkdocs build --strict`.
2. Run `cargo test -p smelt-runtime --test execute_parity` explicitly — phase 8 skipped it
   out of caution and criterion 7 names it, so it must be observed green at HEAD, not
   inferred.
3. Confirm the ratchets are unmoved against their committed baselines:
   `cargo test -p smelt-core --test hardening_budget`,
   `bash .claude/scripts/large-file-check.sh`,
   `git diff --stat main -- .claude/*baseline*` (every bump must be traceable to a
   sign-off note in a phase summary; an untraceable bump is a finding).
4. Close the one docs asymmetry: add the `smelt explain <external step>` form to
   `docs-site/docs/reference/cli.md`'s explain section, cross-linked to
   `guide/external-steps.md`, and add the gate test from Tests above.
5. Append a `## Findings handed back from 20260906-external-dag-steps` section to
   `docs/handoffs/2026-09-08-github-activity-findings.md` recording, with enough context to
   act on without this outcome's summaries: (a) the loader contract as it actually shipped —
   `examples/github_activity/load_day.sh`, declared by
   `models/sources/raw/github_loader.yml`, idempotent per day via its own `main._loader_days`
   ledger, invoked by `smelt run`; (b) `smelt list --format json` hard-failing with
   `ListError::ParseErrors` on three example workspaces (phase 7's discovery — pre-existing,
   unrelated, with the two candidate fixes it named); (c) that a dry run and any embedder
   setting `invoke_external_steps: false` *refuses* when selection reaches a step, which the
   spine's live-BigQuery phases will hit the moment they preview a plan; (d) the unimplemented
   `smelt-ui` plan-preview opt-out phase 4 left.
6. Record the two out-of-scope residues that belong to nobody yet in `docs/TODO.md`: the
   `smelt list` parse-error scoping bug, and `concepts/incremental-equivalence.md` missing
   from the docs-site nav.
7. Judge the criteria. All met → in `outcome.md`, flip row 9 to `done`, set
   `**Status:** done`, and append a dated Decision-log line citing the evidence table. Any
   criterion not met → do not claim it: record what is missing in the summary, leave
   `**Status:** active` and add a phase row for the gap (work serving a success criterion is
   never deferred out).
8. Write `phases/09-summary.md`: the criterion → evidence table, the ratchet check, what was
   handed back and where, and anything a reader six months on would need.

## Verification

- `bash .claude/scripts/verify-phase.sh` — must be ALL GREEN.
- `cargo test -p smelt-runtime --test execute_parity`
- `cargo test -p smelt-core --test hardening_budget`
- `bash .claude/scripts/large-file-check.sh`
- `cargo test -p smelt-cli --test external_step_docs_freshness --test explain_external_step --test list_external_step --test cli_docs_coverage --test example_diagnostics`
- `cargo test -p smelt-lsp --test example_workspaces`
- `cd docs-site && uv run mkdocs build --strict`

## Commit message

`outcome(external-dag-steps): close out — criterion evidence verified at HEAD, findings handed to the dogfood spine`
