# Phase 2 summary — every explain excerpt in the docs-site carries the headline

**Shipped:**
- `crates/smelt-cli/tests/explain_docs_freshness.rs` (new): a standing gate with three tests —
  `every_maintenance_plan_excerpt_leads_with_the_headline` (scans every `docs-site/docs/**/*.md`
  fenced block containing a `Maintenance plan:` line and asserts its first non-`$`-prompt content
  line is `model <name>  (emits: …)`), `cli_reference_sample_matches_real_explain_output` (byte-pins
  `reference/cli.md`'s `smelt explain daily_events` sample against a real CLI run), and
  `incremental_guide_headline_matches_real_explain_output` (pins `guide/incremental-models.md`'s
  un-elided prefix — headline + `Maintenance plan:` line + first cell's opening fields — against a
  real run for `daily_events_enriched`).
- `docs-site/docs/reference/cli.md`: `smelt explain daily_events` sample regenerated from real
  output — now leads with the headline, and picks up fields (`contract:`, `region key:`,
  `admissible write patterns:`, `write pin:`, `Probes (0):`) the binary already prints today that
  the stale sample predated.
- `docs-site/docs/guide/incremental-models.md`: the `daily_events_enriched` excerpt's un-elided
  prefix updated to lead with the headline and `Cells (5):` (was `(4)` — a `raw.users` NewData
  cell was added upstream of this phase); the `...` elisions and surrounding prose untouched.
- Ran `python3 examples/web_analytics/generate_tutorial.py`: no diff (phase 1 already regenerated
  `deduplication.md`, the only page whose explain output changed).

**Decisions:**
- Parsed fenced blocks by tracking `` ``` `` open/close lines rather than `str::split("```")` —
  a naive split treats the fence's language tag (e.g. `text`) as the block's first content line,
  which false-flagged `deduplication.md` and `reference/cli.md`'s already-correct blocks. Recorded
  as `fenced_blocks()` in the new test file.
- The un-elided-prefix comparison in `incremental_guide_headline_matches_real_explain_output`
  compares every committed line up to the *first* `...` marker (not per-cell) — simpler than
  tracking cell boundaries and sufficient for the plan's stated scope (headline + `Maintenance
  plan:` line + first cell).

**For the next planner:**
- No new gaps surfaced beyond what phase 1 already recorded. Phase 3 (rewrite
  `guide/incremental-models.md` around delta signatures, purge four-corners text) and phase 4
  (rename `backbuild-synthesis.md`) are unaffected by this phase's edits — this phase only touched
  the two excerpt blocks and left surrounding prose alone, as planned.
- The `Cells (4)` → `(5)` count drift (an extra `raw.users` NewData cell, unrelated to this
  outcome) was pre-existing upstream drift this phase's regeneration absorbed; no action needed,
  noting it only so a future reader isn't surprised by the count change in the diff.

**Gates:**
- `cargo test -p smelt-cli --test explain_docs_freshness --test tutorial_freshness --test explain_maintenance --test cli_docs_coverage` — all green (3 + 1 + 35 + N passed).
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full workspace
  test suite, example_diagnostics).
- `git diff --stat docs-site/docs` confined to the two excerpt blocks (`reference/cli.md`
  +12/-0, `guide/incremental-models.md` +4/-1); no diff from tutorial regeneration.
