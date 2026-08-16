# Phase 4 plan — rename the ranged-rebuild verb to `smelt rebuild`

## Objective

Advance success criterion 3: the ranged-rebuild verb ships end to end under its spec name
`smelt rebuild`. This is a hard rename with **no alias** — the project carries no
backward-compatibility constraint, and criterion 3 asks for the verb to be gone from CLI,
`--help`, specs, docs-site, examples, and tests. The `backbuild/` module path and the
"backbuild synthesis" *mechanism* name stay (criterion 3 explicitly permits them); only the CLI
verb and prose that means the CLI verb change.

## Spec delta (spec-first — make these edits before the code)

- `docs/specs/cli.md` — verb table row `smelt backbuild` → `smelt rebuild` (line ~25); section
  heading `### smelt run vs smelt backbuild` → `… vs smelt rebuild` and its prose (~373–377);
  `--dry-run` prose (~393–404, including "a real backbuild would execute them" → "a real
  `rebuild`"); constraint item 7 (~547); the `incremental_models.md` cross-reference line (~619).
- `docs/specs/model_selection.md` (~line 54) — the positional-selector callout: `smelt backbuild`
  → `smelt rebuild` throughout the paragraph.
- `docs/specs/definition_deltas.md` — delete the Known Divergences clause stating "the
  ranged-rebuild verb still ships under the name `smelt backbuild` rather than …" (~line 446).
  Leave the §References `backbuild/` module and research-doc paths untouched.
- `docs/specs/architecture.md` — audit only. Its occurrences (lines ~415, ~424, ~484, ~513) all
  name the mechanism/module ("backbuild option catalogue", "backbuild layer", `backbuild/` path,
  "backbuild script"). Expected outcome: **no edit**. If any occurrence turns out to mean the CLI
  verb, rename that one and say so in the summary.

## Tests (red-green)

1. `crates/smelt-cli/tests/rebuild_dry_run.rs` (git-mv of `backbuild_dry_run.rs`, every
   invocation switched to `rebuild`) — the existing dry-run/chunking assertions must pass against
   the new verb. Red before the CLI rename.
2. `rebuild_dry_run.rs::backbuild_verb_no_longer_exists` — `smelt backbuild <sel> --start … --end
   …` exits non-zero with clap's unrecognized-subcommand error; nothing executes.
3. `rebuild_dry_run.rs::help_lists_rebuild_verb` — `smelt --help` contains `rebuild` and does not
   contain `backbuild`.
4. `rebuild_dry_run.rs::no_backbuild_verb_in_user_docs` — scan `docs-site/docs/**` and
   `docs/specs/**` for the literal `smelt backbuild` and for a fenced-block line starting with
   `backbuild `; assert zero hits. A standing ratchet so the verb cannot creep back. (Mechanism
   strings — `backbuild-synthesis`, "backbuild synthesis", `backbuild/` — are not matched by
   either pattern, so no allow-list is needed.)
5. `crates/smelt-cli/tests/e2e/rebuild_cumulative_e2e.rs` (git-mv of
   `backbuild_cumulative_e2e.rs`, `mod` line in `e2e/main.rs` updated) — the cumulative e2e passes
   driving `rebuild`.
6. `crates/smelt-cli/tests/tutorial_freshness.rs` — its `is_backbuild` / `"backbuild"` subcommand
   matching becomes `rebuild`, and the regenerated `changing-things` page it checks is fresh
   against the renamed directives.

## Tasks

1. Land the spec delta above (cli.md, model_selection.md, definition_deltas.md; architecture.md
   audited).
2. `git mv crates/smelt-cli/tests/backbuild_dry_run.rs → rebuild_dry_run.rs` and
   `tests/e2e/backbuild_cumulative_e2e.rs → rebuild_cumulative_e2e.rs`; update `e2e/main.rs`'s
   `mod`; switch every invocation to `rebuild`; add tests 2–4. Confirm red.
3. Rename the CLI surface: `Commands::Backbuild` → `Commands::Rebuild` (+ its doc comment, which
   is the `--help` text), `BackbuildArgs` → `RebuildArgs`, `commands/backbuild.rs` →
   `commands/rebuild.rs` (`git mv`) with `pub mod` + dispatch-arm updates, `pub async fn
   backbuild` → `rebuild`, and the in-file comments/`info!("Backbuild Summary …")` banner.
4. Sweep the remaining production prose that means the verb: `crates/smelt-cli/src/explain.rs`
   (~1256 doc comment), `crates/smelt-cli/src/reporter.rs` (~92 comment),
   `crates/smelt-runtime/tests/surface_audit.rs` comments (~6, ~74),
   `crates/smelt-cli/tests/explain_show_sql.rs` (~343 doc comment).
5. docs-site: `docs/reference/cli.md` (§`smelt backbuild` heading, the synopsis block, all four
   examples, and the lock/wavefront/retry/run-report/`--dry-run` mentions at ~46, 206, 218, 228,
   304, 306), `docs/guide/incremental-models.md` (~705), `docs/developing/architecture.md` (~71
   crate table). Leave `guide/backbuild-synthesis.md` and its `mkdocs.yml` nav entry alone —
   phase 8 rewrites that page in place.
6. Examples: edit `examples/web_analytics/tutorial_pages/changing-things.md` (prose +
   `smelt-generate` directives) and `examples/web_analytics/README.md` (~575) — **templates, not
   the generated docs-site page** — plus `generate_tutorial.py`'s `is_backbuild`/`"backbuild"`
   subcommand handling; then regenerate the tutorial pages with the generator so
   `docs-site/docs/examples/web-analytics/changing-things.md` matches.
7. `README.md` (~57) CLI feature line: `backbuild` → `rebuild`.
8. Update `docs/ROADMAP.md` with the rename, and add a dated Decision-log line to the outcome
   recording the no-alias choice.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test rebuild_dry_run --test tutorial_freshness --test
  explain_show_sql --features duckdb 2>&1 | tail -40`
- `cargo test -p smelt-cli --test e2e --features duckdb 2>&1 | tail -20`
- `cargo test -p smelt-runtime --test statement_parity --test surface_audit 2>&1 | tail -20`
- `rg -n 'smelt backbuild|Backbuild(Args)?|commands::backbuild' crates docs/specs docs-site/docs
  examples README.md` — expect zero hits (module-path/mechanism strings do not match).

## Commit message

`refactor(cli): rename the ranged-rebuild verb from backbuild to rebuild`
