# Phase 8 plan — docs-site page for external steps

## Objective

Land the user-facing documentation for external steps: a Guide page covering the
declaration, the contract (what smelt guarantees and what it does not) and the failure
modes, a key-table section in the Source YAML reference, cross-links from the sources
guide, and a nav entry. Advances success criterion 6's docs half (the fixture half landed
in phase 7); criterion 7's gates stay green.

## Spec delta

None. The normative surface is already in `docs/specs/sources.md` §"Externally-produced
sources (black-box steps)" (phase 1), `docs/specs/cli.md` (phase 6) and
`docs/specs/run_state.md` (phase 5). This phase is user docs only — it restates that
surface in guide voice, adds nothing new, and must be written to the timeless-oracle rule
(no phase/plan vocabulary, no "now supports").

One user-doc **correction** is in scope and is not a spec change:
`docs-site/docs/guide/sources.md` §"Loading source data" currently states flatly that
"smelt does not load source data" — true for a plain source, wrong as an absolute now that
a source may have a step behind it. It must be rewritten to state the default and point at
the new page.

## Tests

New gate `crates/smelt-cli/tests/external_step_docs_freshness.rs` (modelled on
`state_docs_freshness.rs` — plain `fs` reads over `docs-site/docs`, no warehouse):

1. `external_steps_guide_page_exists_and_is_in_the_nav` — `guide/external-steps.md` exists
   and `mkdocs.yml`'s nav references it exactly once.
2. `guide_page_documents_every_declaration_key` — the page names `produces`, `command`,
   `cadence`, `description` and both placeholders `{run_date}`/`{run_end}`.
3. `guide_page_documents_every_failure_mode` — the page names `ExternalStepFailed`,
   `ExternalStepNotInvocable` and `MalformedExternalStep`.
4. `guide_page_states_what_smelt_does_not_do` — the page states smelt never authors,
   parses or type-checks the command, and guarantees no idempotence/retries of it.
5. `sources_guide_no_longer_claims_smelt_never_loads_sources` — the flat
   "smelt does not load source data" sentence is gone and §"Loading source data" links to
   `external-steps.md`.
6. `sources_yml_reference_documents_the_external_step_block` — `reference/sources-yml.md`
   has an `external_step:` section with the four keys and states `columns:` is forbidden
   alongside it.
7. `external_step_docs_carry_no_plan_vocabulary` — no `Phase [A-Z0-9]` match in the new or
   edited docs-site pages (timeless-oracle rule).
8. `external_step_docs_links_resolve` — every relative markdown link in the new page
   resolves to a file under `docs-site/docs`.

## Tasks

1. Write the failing gate `crates/smelt-cli/tests/external_step_docs_freshness.rs` (tests 1-8).
2. Write `docs-site/docs/guide/external-steps.md`: what an external step is; the
   `external_step:` YAML with the `github_activity` loader as the worked example; the four
   keys; the placeholder grammar; what smelt guarantees (DAG ordering ahead of every
   consumer, invocation, failure propagation, reporting in the run report, `smelt explain`
   rendering); what it explicitly does not (authorship, parsing the command, retries,
   idempotence — the loader carries its own bookkeeping, as `examples/github_activity/load_day.sh`
   does); the three failure modes and what a user sees for each; a short "when to use a
   step vs. an out-of-band pipeline" note.
3. Add an `### The `external_step:` block` section to `docs-site/docs/reference/sources-yml.md`
   with the key table and the `columns:`-forbidden rule, linking to the guide page.
4. Rewrite `docs-site/docs/guide/sources.md` §"Loading source data" and add the new page to
   its §"Further reading"; add a pointer from `reference/cli.md`'s `smelt explain` material
   only if `smelt explain <step>` is not already reachable from there.
5. Add `External Steps: guide/external-steps.md` to `mkdocs.yml` nav directly after
   `Sources: guide/sources.md`.
6. Run the gates; fix until green.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test external_step_docs_freshness`
- `cargo test -p smelt-cli --test docs_front_door --test cli_docs_coverage --test explain_docs_freshness --test state_docs_freshness`
- `cd docs-site && uv run mkdocs build --strict` (the CI docs gate; catches a broken link
  or a page missing from the nav). If `uv` is unavailable in this environment, say so
  explicitly in the summary rather than reporting the gate as passed.
- Hardening/large-file ratchets: expected unmoved (docs + one new test file only).

## Commit message

`docs(sources): document external steps — declaration, contract and failure modes`
