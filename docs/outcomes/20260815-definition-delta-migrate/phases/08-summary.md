# Phase 8 summary — docs-site migration guide + the `smelt migrate` doc sweep

## Shipped

- `docs-site/docs/guide/backbuild-synthesis.md` rewritten in place: dropped the "no CLI command"
  Availability warning; added a "Using it" section with real `smelt migrate` / `--apply` / `--json`
  transcripts (verified against `crates/smelt-cli/src/commands/migrate.rs`'s actual output shape);
  added a "Verdicts and approval" section naming all four verdicts (eclipsed, backfill in place,
  rederive, skeleton change) and explaining the per-target/per-model approval store and stale-plan
  refusal; reworded "Current scope" to state `smelt migrate` picks the first admissible technique
  per group with no cost model, rather than claiming it "does not yet choose" (false — it always
  prefers a targeted script to full refresh).
- Cross-links: `docs-site/docs/reference/cli.md` §"smelt migrate" now links to the guide (was
  prose-only "See the migration guide"); `guide/schema-evolution.md` "Further reading" now links
  to the guide (the reverse direction, `incremental-models.md`, already linked).
- Spec edits (status-only): `docs/specs/definition_deltas.md` §References "User docs" now points
  at the guide + `cli.md`; `docs/specs/seeds.md` and `docs/specs/models.md` no longer claim
  `smelt migrate` doesn't exist — reworded to state its actual scope (table migration only, no
  frontmatter rewrite / no seed mode).
- New tests: `page_has_no_stale_availability_wording`, `page_documents_the_migrate_verb`,
  `page_names_every_verdict` in `crates/smelt-logical/tests/backbuild_docs.rs`;
  `no_stale_no_migrate_command_claim` grep-gate in `crates/smelt-db/tests/maintenance_diagnostics.rs`
  (mirrors phase 7's `no_stale_skeleton_column_added_spelling` pattern).

## Decisions

- Kept "enumerates options" framing but corrected it: `smelt migrate` *does* choose (first
  admissible option per group, verified in `crates/smelt-logical/src/backbuild/plan.rs`
  `all_rerun_safe`/`MigrationPlan` — no cost-model comparison exists). Stated precisely rather than
  deleting the caveat outright.
- Verdict label "rederive" (no hyphen) used throughout, matching the CLI's actual
  `MigrationVerdict::Rederive => "rederive"` string, not the outcome-table's "re-derive" spelling.

## For the next planner

- Criterion 18's docs-site CLI-surface audit (this phase's task 8): enumerated every `smelt`
  subcommand (`crates/smelt-cli/src/main.rs` `enum Commands` — init, run, rebuild, table, ui,
  seed, build, type, status, history, explain, bakeoff, diff, migrate, test, check, list, clean,
  docs {generate,list,show,path}) and every `RunArgs`/global flag against `cli.md` — **all are
  documented**, including `--scope`/`--log-format` globals. No gap found; criterion 18 needs no
  further docs-site work for CLI surface (its other sub-items — out-of-band-edit tripwire,
  `on_column_add` supersession, group-merge-provenance, `change_feed` `UpstreamMutation` — are
  design-question work, not docs, per phase 18's own row).
- Not investigated: whether `docs-site/docs/models.md` and `docs-site/docs/seeds.md` (top-level,
  distinct from `docs/specs/`) also carry stale "no smelt migrate" wording — the plan's task 7 and
  the new grep-gate scope only `docs/specs/`. A quick follow-up grep is cheap if this matters.

## Gates

- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy both feature sets, full workspace
  test, example_diagnostics)
- `cargo test -p smelt-logical --test backbuild_docs` — 7/7 pass
- `cargo test -p smelt-db --test maintenance_diagnostics` — 9/9 pass
- `rg -n 'smelt backbuild' docs-site/docs/` — no matches
- `cd docs-site && uv run mkdocs build --strict` — exit 0 (pre-existing INFO-level anchor
  warnings unrelated to this phase's edits, none naming `backbuild-synthesis.md` or
  `schema-evolution.md`)
