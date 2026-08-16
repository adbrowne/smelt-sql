# Phase 10 plan — docs-site migration guide

## Objective

Advance success criterion 7 (and the docs half of 1–3): rewrite
`docs-site/docs/guide/backbuild-synthesis.md` in place so it documents the shipped
`smelt migrate` / `--apply` workflow instead of claiming there is no CLI command, give the verb a
`## smelt migrate` section in `docs-site/docs/reference/cli.md` so the guide's links resolve, and
reword the two spec bullets (`models.md`, `seeds.md`) that still assert `smelt migrate` does not
exist — to say precisely what the verb that *does* exist covers, and what it doesn't.

## Spec delta (made first)

- `docs/specs/seeds.md` §Known Divergences, "Migration tooling" bullet — reword: `smelt migrate`
  exists but is the *definition-delta* verb (migrating a deployed table after its model's SQL
  changes); it does not rewrite seed files or `smelt.yml` seed config. The seed-migration assist
  remains a follow-up.
- `docs/specs/models.md` §"Constraint violations" table row for
  `refresh: batched|keyed|cumulative|versioned` — drop the "(`smelt migrate` applies it)"
  parenthetical: that verb migrates deployed tables, not model config spellings; the fix-it is
  applied by hand.
- `docs/specs/models.md` §Known Divergences, the trichotomy bullet's tail sentence "The
  `smelt migrate` assist for the hard cut does not exist." — reword to name the *config-rewrite*
  assist explicitly (so it does not read as denying the shipped verb) and keep it as the open gap.

## Tests

1. `crates/smelt-cli/tests/rebuild_dry_run.rs::migrate_verb_is_documented` (new, standing docs
   ratchet, sibling to `no_backbuild_verb_in_user_docs`) — asserts (a) the guide mentions
   `smelt migrate` and `--apply`, (b) `docs-site/docs/reference/cli.md` contains a
   `## smelt migrate` heading, and (c) no file under `docs-site/docs` or `docs/specs` claims the
   verb is absent (regex over `No .smelt migrate. command exists` / `no CLI command for this yet`).
   Red before the rewrite, green after.
2. `cargo test -p smelt-logical --test backbuild_docs` (existing, must stay green) — the rewrite
   must preserve every `<!-- backbuild-example(<id>): … -->` marker and its fenced ```sql content
   verbatim; `registry_matches_guide_markers` fails on any dropped or renamed id, and
   `every_script_block_is_marked` fails on any *new* unmarked ```sql fence.
3. `cargo test -p smelt-cli --test rebuild_dry_run no_backbuild_verb_in_user_docs` (existing) — the
   rewrite must not reintroduce the retired verb spelling.

## Tasks

1. Apply the three spec edits above (spec-first).
2. Add `## smelt migrate` to `docs-site/docs/reference/cli.md`, placed after `## smelt rebuild`:
   synopsis (`smelt migrate <model> [--apply] [--json]`), what the plan step prints, the plan-hash
   approval contract, the exit-code contract (`0` no/eclipsed delta or successful apply, `3`
   pending-and-unapproved / stale hash / refused-to-execute, `2` usage), the approval store path
   `.smelt/targets/<target>/migration-approvals.json`, per-column-group resume, and a one-line
   "disjoint from `smelt rebuild`" pointer. Mirror `docs/specs/definition_deltas.md` §Surface
   "`smelt migrate`" and `docs/specs/cli.md` §"Exit codes" — do not invent surface.
3. Rewrite the guide's head matter: delete the `!!! warning "Availability"` admonition and the
   `!!! note "Naming: two things called “backbuild”"` admonition; replace the latter with one prose
   sentence in the intro distinguishing the *data*-side `smelt rebuild` from this *definition*-side
   verb, and add a short "Running a migration" section right after "The idea in one example":
   `smelt migrate <model>` → read the printed plan → `smelt migrate <model> --apply`, with the
   hash-approval rule stated in one sentence and a link to the new CLI anchor.
4. Add a "What `--apply` will and won't execute" section (before "When smelt refuses"): the
   first-presented-candidate rule, one transactional group per column group, the
   all-groups-admitted-before-anything-executes rule, and the three refuse-to-execute cases
   (skeleton change, no admissible candidate, destructive column drop) each pointing at the honest
   route (`smelt build --full-refresh` / `smelt rebuild`). Add a one-paragraph "Resume and CI"
   note (per-group resume; `--json` + exit `3` as the CI gate).
5. Rewrite "Current scope" to match reality: drop "smelt enumerates options; it does not yet
   choose" in favour of "`--apply` executes the first presented candidate; a cost-model chooser is
   future work"; keep the DuckDB-dialect and drop-gating bullets; note the two live narrowings
   (per-group resume, refused destructive legs) matching the spec's Known Divergences.
6. Update "Related pages" and `docs-site/mkdocs.yml`'s nav label (path unchanged:
   `guide/backbuild-synthesis.md`) plus the cross-link sentence in
   `docs-site/docs/guide/incremental-models.md:716` if the page title changes.
7. Any new CLI/console example in the guide uses a ```text or ```console fence — never ```sql —
   so `every_script_block_is_marked` stays satisfied without exemption markers.
8. Run the gates; fix fallout.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test backbuild_docs --quiet`
- `cargo test -p smelt-cli --test rebuild_dry_run --quiet`
- `cd docs-site && mkdocs build --strict 2>&1 | tail -20` (link/anchor integrity for the new
  `#smelt-migrate` anchor and the retitled page) — if `mkdocs` is unavailable, grep the changed
  links against their target headings instead and say so in the summary.

## Commit message

`docs(migrate): document smelt migrate and rewrite the backbuild-synthesis guide around it`
