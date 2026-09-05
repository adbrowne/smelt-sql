## Drift Report: property_diff

**Spec**: docs/specs/property_diff.md (last_reviewed: 2026-09-06)
**Date**: 2026-09-06

### Automated checks

- `cargo fmt --all -- --check` — PASS
- `bash .claude/scripts/clippy-gate.sh` (both CI feature sets) — PASS, zero warnings
- `cargo test -p smelt-db --test integration diagnostics_catalogue` — PASS (1/1)
- `cargo test -p smelt-cli --test property_profile_parity --test property_diff_cli --test property_diff_ci_docs --test cli_docs_coverage` — PASS (3 + 3 + 16 + 4)
- `cargo test -p smelt-logical --test diff_purity --test walk_coverage` — PASS (1 + 8)
- `cargo test -p smelt-core --test baseline --test hardening_budget` — PASS (20 + 4)
- `cargo test -p smelt-runtime --test execute_parity --test profile_workspace` — PASS (4 + 3)
- `cargo test -p smelt-lsp --test property_diff_parity --test property_diff_coalescing --test property_diff_refresh --test property_diff_overlay --test example_workspaces` — PASS (35 + 1 + 1 + 1 + 2)
- `cargo test -p smelt-cli --test example_diagnostics` — PASS (121/122, 1 pre-existing ignore)
- `cd docs-site && mkdocs build --strict` — PASS (pre-existing INFO-level anchor notices only, none touching property-diff pages; `--strict` fails only on WARNING)
- `cargo test --workspace` (full suite) — **not run in this session** per the phase-8 gate ("Another Claude session may be building… do NOT run `cargo test --workspace`; the controller runs it"). Every property-diff-specific and cross-cutting standing gate named in criterion 9 was run individually above and is green; the full-suite run is deferred to the controller.

### Surface drift

- ✅ `--diff [<ref>]`, `--json`, `--markdown`, `--fail-on`, `--select`, `--project-dir` all implemented in `crates/smelt-cli/src/commands/explain_diff.rs`, matching §Surface's table; flag exclusivity with `<model>`/`--show-sql`/`--period`/`--technique` exits `2` (`property_diff_cli.rs`).
- ✅ Text/JSON/Markdown output forms match §"Output forms" — verified against `diff_render.rs` (`text_report`, `markdown_report`) and the JSON schema test in `property_diff_cli.rs`.
- ✅ Marker `<!-- smelt-property-diff -->`, `<details open>` on a downgrade-carrying block, 50-model cap — `MARKER`, `MARKDOWN_MAX_FULL_MODELS` in `diff_render.rs`; `property_diff_ci_docs.rs`.
- ✅ Editor code lens, `PropertyDowngrade` diagnostic, hover — `crates/smelt-lsp/src/property_diff.rs`, `backend.rs`; `property_diff_parity.rs`.
- ⚠️ **Editor "Executing the lens opens the text report… in the editor's output channel"** (§Surface "Editor") has no emission site in any editor — recorded, correctly, as a Known Divergence rather than left silently unmet. Criterion 7 is graded **partially met** below for exactly this reason (see R6 in the phase-8 controller rulings).
- ✅ Diagnostics catalogue: `PropertyDowngrade`, `PropertyDiffBaselineUnavailable` present in `docs/specs/diagnostics.md` §Surface table and `docs-site/docs/reference/diagnostics.md`; the stale "specified and unimplemented" Known Divergences bullet for both codes has been deleted (both have live emission sites).
- ✅ `docs-site/docs/reference/smelt-explain.md` and `docs-site/docs/guide/ci.md` describe the CLI/CI surface; `docs-site/docs/guide/editor-features.md` now correctly describes the open-buffer overlay (previously said the opposite — fixed this phase).

### Semantics drift

- ✅ "The property profile" (§Semantics) — `PropertyProfile { properties, cell_verdicts, refusals, probes }` in `crates/smelt-logical/src/analysis/profile.rs`; covered by `property_profile_parity` (byte-identity vs the single-version report).
- ✅ "The diff" — added/removed/unshifted/shifted rules, per-field matching keys, `maintenance_lost`/`maintenance_gained` — covered by `crates/smelt-logical/src/analysis/diff.rs`'s own unit tests (51+ under `cargo test -p smelt-logical --lib analysis::diff`, not re-run this session but unchanged since phase 3/5) and the real-fixture tests in `property_diff_cli.rs`.
- ✅ "Direction" table — exhaustive match with no wildcard arm (`ChangeKind::direction`), each row covered by a unit test per phase 3's plan.
- ✅ "Attribution" — nearest-edited-ancestor walk, `of: []` project-config case — `property_diff_cli.rs`'s `a_join_induced_downgrade_propagates_to_the_named_downstream_model`.
- ✅ "Baseline materialisation" — `git archive` into scratch, `load_workspace`, cleanup — `crates/smelt-core/src/baseline.rs`; `baseline.rs` test `diff_leaves_no_repository_state`.
- ✅ §Constraints item 1 (profile single ownership) — `smelt-runtime::build_model_diagnostics` is the one assembler consumed by both the CLI report and the diff (`crates/smelt-runtime/src/profile.rs`, `property_diff.rs`).
- ✅ §Constraints item 2 (diff purity) — `diff_purity.rs` asserts no I/O.
- ✅ §Constraints item 5 (surface parity) — `property_diff_parity.rs` compares LSP lens/diagnostic counts against the CLI JSON for the same workspace/ref, proven non-vacuous by phase 7's sabotage run (documented in `08-plan.md`/`07-summary.md`).
- ⚠️ The LSP refresh coalescer's `pending`-trailing-rerun path (a second trigger arriving mid-refresh) has no dedicated race-inducing test — recorded as a new Known Divergence this phase, not previously named in the spec.

### Invariant drift

- ✅ Constraint 3 (direction totality) — `Dimension`/`ChangeKind` exhaustive matches, verified by inspection (`diff.rs`); a missing arm is a compile error.
- ✅ Constraint 6 (fail-loud) — unresolvable baseline is `PropertyDiffBaselineUnavailable`/exit 2 (`baseline.rs`, `explain_diff.rs`), never an empty diff; a per-side derivation failure carries its reason in `cause.reason` rather than being silently dropped.
- ✅ Constraint 7 (loading parity) — both sides load through `load_workspace` (`crates/smelt-core/src/workspace.rs`), no second discovery path.
- ✅ Constraint 8 (no repository mutation) — `baseline.rs`'s `diff_leaves_no_repository_state` asserts `git status --porcelain` and `git worktree list` are unchanged.
- ✅ Constraint 9 (append-stable JSON) — not independently re-verified this session beyond the existing schema test; no field was removed or renamed by this phase's doc-only changes.

### Timeless-oracle drift

- ✅ No phase-vocabulary leakage detected in `docs/specs/property_diff.md`, `docs/specs/diagnostics.md`, `docs/specs/cli.md`, `docs/specs/lsp.md` body sections, or the four `docs-site/` pages referenced by §References → User docs. `grep -nE 'Phase [A-Z0-9]+'` hits only the meta-description of the timeless-oracle rule itself in each spec's own front-matter callout, not actual phase labels.
- Note: this validation, like `/smelt:validate`'s own step 5, scans only the spec file and the §References → User docs paths — it does not scan `docs/outcomes/`, where phase vocabulary is correctly in scope and untouched (per phase-8 ruling R5).

### Freshness

- `last_reviewed`: 2026-09-06 (bumped this phase)
- Most recent code change touching §References → Code paths: 2026-09-06T02:49:59+10:00 (this phase's own edits — comment-only in `crates/smelt-lsp/tests/property_diff_coalescing.rs`)
- Verdict: fresh.

### Summary

- Drift items found and **fixed this phase**: stale diagnostics.md bullet (deleted), §References citing a nonexistent test file and wrong module names (rewritten), a never-written plan-file citation (repointed to the outcome), the Overview example's self-inconsistent counts/reason placement (corrected), `editor-features.md`'s inverted open-buffer claim (corrected), a tower-lsp "multi-FileEvent truncation" claim in `property_diff_coalescing.rs`'s comment that did not reproduce under direct evidence (softened, evidence recorded in `crates/smelt-lsp/CLAUDE.md`), a missing hardening-baseline sign-off line (added).
- Drift items remaining, named as Known Divergences rather than silently closed: the lens-action no-op (criterion 7 partially met), no example fixture for `state_downgrade` or a purely combiner-driven downgrade, the refresh coalescer's untested trailing-rerun race, the open `column_added` direction question, the uniform baseline-failure-silence posture, the three uncoded admission refusals, the `EntityKind::Model` DDL-misclassification edge case.
- Recommended next step: none required for closure — every remaining item has a named tracking link (`docs/outcomes/20260905-property-diff/outcome.md`) and is not itself spec-code disagreement, only incomplete surface (the lens action) or missing fixture coverage.
