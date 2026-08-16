# Phase 10 summary — docs-site migration guide

**Shipped:**
- `docs-site/docs/reference/cli.md` gained a `## smelt migrate` section (after `## smelt rebuild`):
  synopsis, flags, plan/approve/execute/resume contract, exit codes, examples — mirroring
  `docs/specs/definition_deltas.md` §Surface and `docs/specs/cli.md` §"Exit codes".
- `docs-site/docs/guide/backbuild-synthesis.md` rewritten in place: dropped the "Availability"
  warning and the "two things called backbuild" admonition (replaced with one intro sentence
  linking `smelt migrate`); added "Running a migration" (right after the intro example) and
  "What `--apply` will and won't execute" (before "When smelt refuses"); "Current scope" reworded
  to state the first-presented-candidate rule, the per-group resume narrowing, and that a
  destructive candidate refuses rather than "backbuild only orders it"; "Related pages" links the
  new CLI section.
- `docs/specs/seeds.md` and `docs/specs/models.md`: the two "`smelt migrate` doesn't exist"
  bullets reworded to name what the shipped verb (deployed-table migration) does and doesn't
  cover — `models.md`'s stale "(`smelt migrate` applies it)" fix-it parenthetical dropped.
- New standing ratchet `crates/smelt-cli/tests/rebuild_dry_run.rs::migrate_verb_is_documented`
  (sibling to `no_backbuild_verb_in_user_docs`): guide mentions `smelt migrate`/`--apply`, the CLI
  reference has the `## smelt migrate` heading, and no doc under `docs-site/docs`/`docs/specs`
  still claims the verb doesn't exist.

**Decisions:**
- Page title (`# Backbuild Synthesis`) and file path stay unchanged — the plan's title-change
  condition for updating `mkdocs.yml`'s nav label and `incremental-models.md:716`'s cross-link
  didn't apply, so neither was touched.
- Kept every `<!-- backbuild-example(id): ... -->` marker and its fenced ```sql content verbatim;
  all new CLI examples use ```console fences.

**For the next planner:**
- Phase 9's two carry-forwards (double-derivation collapse, LSP staleness follow-up already
  scoped in) are untouched by this phase — still open, not blocking criterion 7.
- `mkdocs build --strict` surfaces several pre-existing broken anchors (meta-language `reference.md`,
  `reference/cli.md`'s own dry-run/jobs/resume/upstream anchors, `smelt-yml.md`) unrelated to this
  phase's edits — worth a cleanup pass but out of this outcome's scope.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy, full `cargo test`, example_diagnostics)
- `cargo test -p smelt-logical --test backbuild_docs --quiet` — 4 passed
- `cargo test -p smelt-cli --test rebuild_dry_run --quiet` — 5 passed (including new ratchet)
- `cd docs-site && uv run mkdocs build --strict` — exit 0; no new-link warnings (pre-existing
  unrelated anchor warnings only)
