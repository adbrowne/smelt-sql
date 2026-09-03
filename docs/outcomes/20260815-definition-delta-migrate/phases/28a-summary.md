# Phase 28a summary

**Shipped:**
- `docs/specs/incremental_models.md:1598-1599` — the "no out-of-band-edit tripwire" clause now
  cross-references §"Other deliberate boundaries" (the non-goal), dropping the stale "an
  explicit Open Question, §Known Divergences" framing.
- `docs/specs/definition_deltas.md` §Design — new paragraph recording that a per-model
  `on_column_add: backfill | leave_null | recompute` policy knob was considered and dropped
  (verified no live spec or docs-site page mentions it before writing the rejection).
  `last_reviewed` bumped to 2026-09-03.
- `docs/specs/incremental_models.md` §Known Divergences — deleted the "docs-site coverage of
  the plan's CLI surface is partial" bullet.
- `crates/smelt-cli/tests/cli_docs_coverage.rs` (new) — three tests
  (`every_command_is_documented`, `every_long_flag_is_documented`,
  `allowlist_has_no_stale_entries`) walking `Commands`/`DocsCommands`/`*Args` structs in
  `main.rs` via source-text brace-counting (no clap introspection needed — `Cli`/`Commands`
  live in the binary, not the crate's `lib.rs`) against `docs-site/docs/reference/cli.md`.

**Decisions:**
- Ran the coverage tests red-first with an empty `UNDOCUMENTED_BY_DESIGN` allowlist: the audit
  found **zero residue** across all 22 subcommands (including nested `docs generate/list/show/
  path`) and ~90 long flags. cli.md was already comprehensive, so the divergence bullet was
  deleted outright instead of narrowed to a residual list.
- The check is deliberately flag-text-containment (`--flag` appears verbatim anywhere in
  cli.md), not scoped to the flag's own `## smelt <cmd>` section — the plan's wording ("must
  appear verbatim in cli.md") supports this, and it correctly treats flags documented once
  under "Common Flags" (`--project-dir`, `--target`, `--select`, …) as covered everywhere they
  recur, matching how the page is actually organized.
- Corrected the plan's verdict-name shorthand (`PureBackfill`/`UpstreamRederive`/`SkeletonAdd`)
  to the real names: the spec's own prose terms (backfill in place / re-derive / skeleton
  change) and the actual Rust enum (`MigrationVerdict::{Eclipsed,BackfillInPlace,Rederive,
  SkeletonChange}` in `crates/smelt-logical/src/backbuild/plan.rs`) don't match the plan's
  guessed names — verified before citing either.

**For the next planner:**
- The coverage gate is a name/flag *presence* check only — it can't catch a flag whose
  documented behavior has drifted from its actual semantics. That's out of scope for this
  phase and not a gap the gate claims to close.
- No new residue to schedule; the CLI-coverage divergence is fully closed.

**Gates:**
- `cargo test -p smelt-cli --test cli_docs_coverage` — pass (3/3).
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace test, example_diagnostics).
- `rg -n "on_column_add|out-of-band" docs/specs docs-site/docs` — no surviving "(Open
  Question)" framing for either item.
- mkdocs build spot-check skipped (mkdocs not installed in this environment); edits are
  markdown-only with no nav change, consistent with the plan's expectation.
