# Phase 4 plan — Rename `smelt backbuild` → `smelt rebuild`

## Objective

Ship the ranged-rebuild verb under its spec name end to end (success criterion 3): CLI
subcommand, args struct, module and test filenames, `--help`, docs-site pages, the
web-analytics tutorial generator + its generated page, and the spec sweep named in success
criterion 8 (`cli.md`, `model_selection.md`, `architecture.md` prose). The `backbuild/`
crate module and "backbuild synthesis" as the *definition-delta mechanism's* name are
deliberately untouched — only the CLI verb renames.

## Spec delta (do this first)

- `docs/specs/cli.md` — verb table row (25), `### smelt run vs smelt backbuild` heading and
  body (339–343), `--dry-run` paragraphs (359, 367, 370), non-goal 7 (513), and the
  `incremental_models.md` cross-reference (585): all rename to `smelt rebuild`. Prose that
  says "a real backbuild" becomes "a real rebuild".
- `docs/specs/model_selection.md` — line 54 callout renames to `smelt rebuild` (heading,
  body, and both `backbuild` mentions).
- `docs/specs/architecture.md` — 415/424/484 keep `backbuild/` as the **module path** and
  "backbuild option catalogue"/"backbuild synthesis" as the mechanism name (they are not the
  verb); 513's divergence bullet keeps its module/emitter references but its "no CLI/runtime
  consumer drives a backbuild script" sentence is untouched here (phase 8/9 territory).
  Net: a per-mention pass, not a blanket replace — verb ⇒ rename, module/mechanism ⇒ keep.
- `docs/specs/definition_deltas.md` §Known Divergences — delete the bullet at 439 ("The
  ranged-rebuild verb still ships under the name `smelt backbuild`"). Other `backbuild`
  mentions in that file are module paths and stay.
- **No compatibility alias.** `smelt backbuild` stops existing (no hidden alias, no
  deprecation shim) — early-stage project, no back-compat constraint, and an alias would
  keep the collision the rename exists to remove.

## Tests (red-green)

1. `crates/smelt-cli/tests/rebuild_dry_run.rs` (git-mv of `backbuild_dry_run.rs`) — every
   existing dry-run assertion passes when invoked as `smelt rebuild`.
2. `rebuild_dry_run.rs::backbuild_verb_is_gone` (new) — `smelt backbuild <sel> …` exits `2`
   with an unrecognised-subcommand error; proves no alias survives.
3. `rebuild_dry_run.rs::help_lists_rebuild` (new) — `smelt --help` contains `rebuild` and
   not `backbuild`.
4. `crates/smelt-cli/tests/e2e/rebuild_cumulative_e2e.rs` (git-mv of
   `backbuild_cumulative_e2e.rs`) — cumulative e2e passes under the new verb.
5. `cargo test -p smelt-cli --test tutorial_freshness` — the regenerated
   `changing-things.md` matches what the generator produces for the renamed verb.
6. `cargo test -p smelt-cli --test explain_show_sql` — the `--show-sql` comparison against
   the rebuild dry-run path still holds after the rename.

## Tasks

1. Spec sweep above (cli.md, model_selection.md, architecture.md, definition_deltas.md).
2. `crates/smelt-cli/src/main.rs`: `Commands::Backbuild(BackbuildArgs)` →
   `Commands::Rebuild(RebuildArgs)`, doc comment, dispatch arm.
3. `git mv crates/smelt-cli/src/commands/backbuild.rs .../rebuild.rs`; rename
   `pub fn backbuild` → `rebuild`; update `commands/mod.rs`.
4. Update verb-name mentions in `crates/smelt-cli/src/{explain.rs,reporter.rs}` and
   `crates/smelt-runtime/src/{execute.rs,reporter.rs,types.rs,fn_bodies.rs}` doc comments
   (`fn_bodies.rs`'s `commands/backbuild.rs` path reference included).
5. `git mv` the two test files (tests 1, 4); update `tests/e2e/main.rs` module name and all
   in-file argv/identifiers; add tests 2 and 3.
6. `examples/web_analytics/generate_tutorial.py` — `is_backbuild`/`"backbuild"` →
   `is_rebuild`/`"rebuild"`; `examples/web_analytics/tutorial_pages/changing-things.md` and
   `README.md` — rename the verb in the directives/prose.
7. Regenerate `docs-site/docs/examples/web-analytics/changing-things.md` by running
   `generate_tutorial.py` (never hand-edit the generated page); mirror the identifier rename
   in `crates/smelt-cli/tests/tutorial_freshness.rs` (it mirrors the generator by contract).
8. docs-site verb-name sweep: `reference/cli.md`, `guide/incremental-models.md` (705, 716),
   `developing/architecture.md` (71), `README.md` (57). In
   `docs-site/docs/guide/backbuild-synthesis.md`, rename verb mentions and drop the
   "Naming: two things called 'backbuild'" callout (now factually false); its narrative
   rewrite around `smelt migrate` stays phase 8's. Page filename and mkdocs nav entry keep
   the "backbuild synthesis" mechanism name.
9. `rg -n '\bsmelt backbuild\b|Commands::Backbuild|BackbuildArgs'` over the tree
   (excluding `docs/plans`, `docs/outcomes`, `docs/research`, `docs/validations`,
   `docs/handoffs`, `docs/bug-hunt`, `docs/codebase-review-*`, `target`, `docs-site/site`)
   returns nothing; historical docs are left as written.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test rebuild_dry_run`
- `cargo test -p smelt-cli --test tutorial_freshness`
- `cargo test -p smelt-cli --test explain_show_sql`
- `cargo test -p smelt-cli --test e2e` (rebuild_cumulative_e2e leg)
- `cargo test -p smelt-runtime --test surface_audit`

## Commit message

`refactor(cli)!: rename smelt backbuild to smelt rebuild`
