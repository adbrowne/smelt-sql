# Phase 4 summary — rename the ranged-rebuild verb to `smelt rebuild`

**Shipped:**
- CLI: `Commands::Backbuild(BackbuildArgs)` → `Commands::Rebuild(RebuildArgs)`;
  `commands/backbuild.rs` → `commands/rebuild.rs` (`git mv`), `pub async fn backbuild` → `rebuild`.
  No alias — `smelt backbuild` no longer parses.
- Spec sweep: `docs/specs/cli.md` (verb table, `### smelt run vs smelt rebuild` heading + prose,
  `--dry-run` section, constraint item 7, cross-reference line), `docs/specs/model_selection.md`
  (positional-selector callout), `docs/specs/definition_deltas.md` (Known Divergences bullet on
  the still-`backbuild`-named verb deleted — the bullet now leads with the remaining
  column-additions-only gap). `docs/specs/architecture.md` audited: all four occurrences name the
  mechanism/module, no edit.
- docs-site: `reference/cli.md` (full `## smelt rebuild` section + all cross-references),
  `guide/incremental-models.md` (`## Rebuilding` section), `developing/architecture.md` (crate
  table). `guide/backbuild-synthesis.md` structurally untouched (phase 8's page), but its two
  literal `smelt backbuild` mentions (naming-collision callout, related-pages line) were fixed —
  see "For the next planner".
- Examples: `tutorial_pages/changing-things.md` template (prose + `smelt-generate` directives),
  `generate_tutorial.py` (`is_backbuild`→`is_rebuild`, `"backbuild"`→`"rebuild"` dispatch table
  key), `README.md`; regenerated `docs-site/docs/examples/web-analytics/changing-things.md` via
  the generator (only that page changed, as expected).
- Tests: `git mv backbuild_dry_run.rs → rebuild_dry_run.rs` (+2 new tests:
  `backbuild_verb_no_longer_exists`, `help_lists_rebuild_verb`, `no_backbuild_verb_in_user_docs`
  ratchet); `git mv e2e/backbuild_cumulative_e2e.rs → rebuild_cumulative_e2e.rs` (`mod` updated in
  `e2e/main.rs`); `tutorial_freshness.rs`'s mirrored `is_backbuild`/`"backbuild"` logic renamed to
  match the Python generator.
- Production prose sweep: `smelt-cli/src/explain.rs`, `reporter.rs`; `smelt-runtime/src/
  fn_bodies.rs`, `reporter.rs`, `types.rs`, `execute.rs`; `smelt-runtime/tests/surface_audit.rs`;
  `smelt-cli/tests/explain_show_sql.rs`; `smelt-logical/src/backbuild/mod.rs`'s own module doc
  comment (referenced the CLI verb by old name).
- `README.md` CLI feature line.

**Decisions:**
- Hard rename, no alias (already decided in the phase-4 planning entry, 2026-08-16) — executed
  as specified.
- The ratchet test (`no_backbuild_verb_in_user_docs`) only flags a fenced-code-block line starting
  with `backbuild ` (not any prose line that happens to wrap onto `backbuild` at column 0) — the
  plan's literal wording ("a fenced-block line") is load-bearing; an unscoped prose-line check
  produced a false positive on `guide/backbuild-synthesis.md`'s "backbuild only orders the
  statement, never gates it." bullet, a legitimate mechanism reference.

**For the next planner:**
- `guide/backbuild-synthesis.md` had two literal `smelt backbuild` mentions the plan's file list
  didn't call out (a naming-collision admonition and a related-pages cross-reference) — fixed
  minimally (verb name + anchor only) to satisfy the ratchet without doing phase 8's full-page
  rewrite. Phase 8 should treat this page as already verb-clean; its remaining work is the
  content/structure rewrite (removing the whole naming-collision callout, `--apply` framing).
- `docs/specs/architecture.md` line 513 ("no CLI/runtime consumer drives a backbuild script
  through a real backend yet") reads as stale now that `smelt migrate --apply` executes (phase 3)
  — but that line is about the *backbuild-synthesis* mechanism's executed-statement parity gap,
  a different claim than the CLI verb rename this phase covers, and the plan's audit instruction
  said no edit. Flagging for whichever phase (6 or a future one) owns that divergence's accuracy.
- No rework expected for phase 5 (conformance harness definition-edit step kind) — nothing in
  this phase touched maintenance/conformance code paths.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings, full `cargo test`
  workspace, `example_diagnostics`).
- `cargo test -p smelt-cli --test rebuild_dry_run --test tutorial_freshness --test
  explain_show_sql --features duckdb` — 11 passed.
- `cargo test -p smelt-cli --test e2e --features duckdb` — 175 passed.
- `cargo test -p smelt-runtime --test statement_parity --test surface_audit` — 25 passed.
- `rg -n 'smelt backbuild|Backbuild(Args)?|commands::backbuild' crates docs/specs docs-site/docs
  examples README.md` — remaining hits are all `BackbuildInputs`/`BackbuildOptions`/module-path/
  mechanism-name strings (expected per plan) plus the intentional literal-string test fixtures in
  `rebuild_dry_run.rs` itself.
