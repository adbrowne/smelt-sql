# Phase 8 plan — docs-site migration guide + the `smelt migrate` doc sweep

## Objective

Rewrite `docs-site/docs/guide/backbuild-synthesis.md` in place so it documents the shipped
`smelt migrate` / `--apply` workflow instead of announcing that no CLI command exists, and
correct the two sibling-spec bullets that still record `smelt migrate` as absent. Advances
success criterion 7 (a migration guide ships, the placeholder page is rewritten rather than
left beside a new page), the `definition_deltas.md` §References "User docs: none yet" line, and
criterion 8's `models.md` / `seeds.md` bullets. Also lands criterion 18's docs-site
CLI-surface coverage checklist.

## Spec delta

No user-visible behaviour changes. Two spec edits (documentation-status only), made by the
implementer as part of this phase:

- `docs/specs/definition_deltas.md` §References — replace `- **User docs**: none yet — the
  docs-site page for migration lands with the wiring plan.` with a pointer to
  `docs-site/docs/guide/backbuild-synthesis.md` and `docs-site/docs/reference/cli.md`
  §"smelt migrate".
- `docs/specs/seeds.md` line ~180 and `docs/specs/models.md` lines ~244 / ~346 — reword, do not
  delete. `smelt migrate` ships as a *definition-delta table migration* verb only; it does not
  rewrite a retired `refresh:` value in frontmatter, and there is no seed-migration assist. So:
  `models.md` 244 drops the parenthetical claim that `smelt migrate` applies the refresh fix-it
  (the fix-it text is the assist); `models.md` 346's "The `smelt migrate` assist for the hard cut
  does not exist" narrows to say the verb exists but does not cover frontmatter rewrites;
  `seeds.md` 180 narrows from "No `smelt migrate` command exists" to "`smelt migrate` migrates a
  model's stored table after a definition change; it has no seed-migration mode".

## Tests

All in `crates/smelt-logical/tests/backbuild_docs.rs` (the existing doc-sync gate for this page),
except the last:

- `page_has_no_stale_availability_wording` — the page contains none of "no CLI command for this
  yet", "what remains is the surface on top", or a `!!! warning "Availability"` admonition.
- `page_documents_the_migrate_verb` — the page names `smelt migrate`, `smelt migrate --apply`,
  `--json`, exit code `3`, and links to `../reference/cli.md`.
- `page_names_every_verdict` — the four verdict labels the plan prints (eclipsed, backfill in
  place, re-derive, skeleton change) each appear on the page, so the guide covers the surface
  `definition_deltas.md` specifies.
- `every_sql_block_still_carries_a_marker` — already exists; must stay green after the rewrite
  (new CLI transcripts use ```text / ```console fences, or carry
  `<!-- backbuild-example-exempt: ... -->`).
- `crates/smelt-db/tests/maintenance_diagnostics.rs::no_stale_no_migrate_command_claim` — grep
  gate over `docs/specs/` for the literal "No `smelt migrate` command exists" (needle assembled
  from parts so the test's own source doesn't trip it), mirroring phase 7's grep-gate pattern.

## Tasks

1. Red: add the four new tests above; confirm they fail against today's page/specs.
2. Rewrite the page's opening: drop the Availability warning, reframe "before backbuild
   synthesis" prose into present tense, and add a short "Using it" section right after "The idea
   in one example" — `smelt migrate <model>` prints the per-group plan and exits `3`;
   `--apply` executes only the plan whose hash matches the recorded approval; `--json` is the CI
   form. Keep transcripts consistent with `cli.md` §"smelt migrate" (verify against the real
   command output in `crates/smelt-cli/src/commands/migrate.rs`, do not invent formatting).
3. Add a "Verdicts and approval" section mapping the four per-group verdicts to the technique
   tour already on the page, and explaining the approval store (per-target, per-model recorded
   plan hash) and the stale-plan refusal.
4. Update "Current scope" — remove the "enumerates options; it does not yet choose" framing only
   if it is now false; otherwise keep it and state which option `smelt migrate` prints. Verify
   against the code rather than assuming.
5. Confirm the "Naming: two things called 'backbuild'" callout is already absent (phase 4 renamed
   the verb); if any residue remains, remove it. Ensure every `smelt backbuild` mention on the
   page reads `smelt rebuild`.
6. Cross-links: `docs-site/docs/reference/cli.md` §"smelt migrate" gains a link to this guide
   (its prose already says "See the migration guide" without a link); `guide/incremental-models.md`
   and `guide/schema-evolution.md` links checked both directions.
7. Make the three spec-status edits from §Spec delta.
8. Criterion 18's docs-site CLI-surface audit: enumerate, as a checklist appended to this phase's
   summary (not the docs-site), every `smelt` subcommand and every `smelt run` flag in
   `crates/smelt-cli/src/cli.rs` that has no `docs-site/docs/reference/cli.md` entry. Document or
   explicitly drop each; the checklist is the deliverable, not a re-audit.
9. Green: run the gates; `cargo fmt --all`.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test backbuild_docs`
- `cargo test -p smelt-db --test maintenance_diagnostics`
- `rg -n 'smelt backbuild' docs-site/docs/` → no matches
- `cd docs-site && mkdocs build --strict` (if available in this environment; if not, note it in
  the summary rather than skipping silently)

## Commit message

`docs(migrate): rewrite the backbuild-synthesis guide around smelt migrate/--apply`
