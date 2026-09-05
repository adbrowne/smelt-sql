# Phase 8 plan — docs sweep and closure

## Objective

Close the outcome. Every claim the feature's docs make must be true of the code that shipped in
phases 1–7, every deferral recorded in a phase summary must have a durable home in a spec
§Known Divergences (never only in an outcome summary), `/smelt:validate property_diff` must
report zero drift, and all nine success criteria must be checked with named evidence.

No new user-visible behaviour. This is a documentation, spec-hygiene and verification phase; the
only code touched is a possible test-harness fix (task 6) and a baseline comment (task 10).

## Spec delta

`docs/specs/property_diff.md`:
- §Overview worked example — fix two self-consistency defects against the implemented rules
  (see task 1). Non-normative, but it is the first thing a reader sees.
- §Known Divergences — delete nothing that is still true; add the items in task 3; re-point every
  "Tracked in `docs/outcomes/…`" link now that the outcome is complete.
- §References — rewrite Code/Tests/User docs to the paths that exist; replace the never-written
  `docs/plans/20260905-property-diff.md` (task 4).
- Front matter `last_reviewed: 2026-09-05` → `2026-09-06`.

`docs/specs/diagnostics.md`:
- Delete the "`PropertyDowngrade` and `PropertyDiffBaselineUnavailable` are specified and
  unimplemented" bullet (line ~583). Both now have live emission sites.

## Task list

1. **Overview example self-consistency** (`property_diff.md` §Overview). Two defects found:
   (a) the summary line reads `4 downgrades, 0 upgrades, 1 neutral.` but the block above it
   shows **five** `▼` lines — recount or drop a line; (b) the `reason:` line hangs off a
   `cell technique` change, and `Change::reason` is populated for exactly one dimension in
   `crates/smelt-logical/src/analysis/diff.rs:543` (`state_downgrade`) — no technique change can
   ever carry one. Also align the change-line spellings with `diff_render::change_line`
   (`▼ cell_technique revenue@orders: KeyedFold → DeleteInsert`, `▼ source_bound orders: …`) and
   the header with `text_report` (`property diff vs <ref> = <commit> (N file(s) changed,
   M model(s) shifted)`). **On the `SUM → MAX` framing:** the Overview's prose claim ("replacing
   `SUM` with `MAX` … can silently demote a model") is a claim about the *feature class*, not
   about `examples/timeseries`, and is true where a correction/`UpstreamMutation` cell needs an
   invertible combiner. Keep it, but add the qualifier the Decision log established — it is a
   downgrade only where invertibility is load-bearing, never over a `NewData` fold on an
   append-only source — so the example cannot be read as promising a shift that phase 5 proved
   `examples/timeseries` does not produce.
2. **Kill the stale "specified and unimplemented" bullet** in `docs/specs/diagnostics.md`.
   Red-green: `cargo test -p smelt-db --test integration diagnostics_catalogue` must stay green
   (it asserts enum → catalogue coverage, not the divergence prose).
3. **Known Divergences reconciliation.** Audit every one of the five existing bullets, then add
   the ones only recorded in phase summaries:
   - keep: `column_added` neutrality (open question); three uncoded refusals; the
     `EntityKind::Model` DDL-script misclassification; uniform baseline-failure silence in the
     editor; the un-registered lens command.
   - **add** — `CellVerdict.state_downgrade` has a live producer only for a model whose target
     dialect makes it fire (phase 5 C4's dual-target fixture); no `examples/` model exercises it.
   - **add** — `examples/timeseries` has no fixture demonstrating a *combiner-driven* downgrade;
     one needs a model whose driving source is declared mutable (phase 5's explicit recommendation).
   - **add** — the `pending` trailing-rerun path of the LSP refresh coalescer has no
     race-inducing test (phase 7 fix round, "Still open").
   - **add or resolve** — the tower-lsp multi-`FileEvent` finding (task 6).
   Each bullet describes behaviour, not phases, and links `docs/outcomes/20260905-property-diff/outcome.md`.
4. **§References rewrite.** Every current entry is wrong or incomplete. Correct set:
   Code — `crates/smelt-logical/src/analysis/{profile,diff,diff_render}.rs`,
   `crates/smelt-core/src/baseline.rs`, `crates/smelt-core/src/workspace.rs` (open-buffer
   overlay), `crates/smelt-runtime/src/{profile,property_diff}.rs`,
   `crates/smelt-cli/src/commands/explain_diff.rs`, `crates/smelt-lsp/src/property_diff.rs`.
   Tests — `crates/smelt-cli/tests/{property_profile_parity,property_diff_cli,property_diff_ci_docs}.rs`,
   `crates/smelt-logical/tests/diff_purity.rs`, `crates/smelt-core/tests/baseline.rs`,
   `crates/smelt-runtime/tests/profile_workspace.rs`,
   `crates/smelt-lsp/tests/property_diff_{parity,refresh,coalescing,overlay}.rs`.
   User docs — add `docs-site/docs/guide/editor-features.md` alongside the two present.
   **Plans (history)** — there is no plan file and one must not be invented. Replace the line with
   a link to `docs/outcomes/20260905-property-diff/outcome.md`, labelled as the outcome that
   landed the feature. Verify every path with `ls` before committing.
5. **`editor-features.md` open-buffer correction.** The page says an unsaved edit "does not move
   the lens until you save". That contradicts `property_diff.md` §Surface "Editor" ("Open buffers
   override on-disk contents for model files on the working-tree side") and the phase-7 test
   `property_diff_overlay.rs::an_unsaved_buffer_edit_changes_the_lens_and_diagnostics_without_touching_disk`.
   Rewrite: an unsaved edit does not *trigger* a refresh, but the next refresh from any cause
   reads the buffer, not the disk; an unsaved `smelt.yml`/source YAML takes effect only on save.
6. **Adjudicate the tower-lsp multi-`FileEvent` finding** (phase 7 fix round). Bounded
   investigation, no more than one sitting: instrument the harness to dump (a) the exact bytes
   written to the duplex stream and (b) the deserialized `params.changes.len()` inside
   `did_change_watched_files`. If the wire body carries N events and the handler sees 1, it is a
   real transport defect reachable by a real editor → `property_diff.md` §Known Divergences plus
   a note in `crates/smelt-lsp/CLAUDE.md`, describing the user-visible consequence (a multi-project
   workspace could miss refreshing all but the first affected project on one notification). If the
   wire body carries 1 event, it is a harness framing bug → fix the harness if cheap, otherwise
   record it durably in `crates/smelt-lsp/CLAUDE.md` as a known test-harness limitation with the
   evidence. It must not evaporate either way.
7. **Timeless-oracle sweep.** `grep -nE 'Phase [A-Z0-9]+'` over `docs/specs/property_diff.md`,
   `docs/specs/{diagnostics,cli,lsp}.md` and the four `docs-site/` pages — currently zero hits,
   re-confirm after every edit above. Note in the phase summary that `/smelt:validate` step 5
   scans only the spec file and the paths in §References → User docs; it does **not** scan
   `docs/outcomes/`, so this outcome's phase-vocabulary-laden summaries are correctly out of
   scope and must not be sanitised.
8. **ROADMAP entry.** Add to `docs/ROADMAP.md` §"Recently Completed", above the August 24 entry:
   `### ~~Property diff — "explain the diff" for model edits~~ ✅ (September 6, 2026)`, naming the
   surfaces (`smelt explain --diff` text/JSON/Markdown, `--fail-on`, the CI dogfood job, the LSP
   code lens and `PropertyDowngrade`), the standing gates, and the divergences left open.
   Cross-check §"What's Next" for any item this supersedes.
9. **Run `/smelt:validate property_diff` and fix what it reports.** Expect hits on §References
   (task 4), the diagnostics.md bullet (task 2), and the `editor-features.md` open-buffer line
   (task 5) if run before them; run it *after* tasks 1–8 so the report is the closure evidence.
   Persist it to `docs/validations/2026-09-06-property_diff.md`.
10. **Hardening-baseline sign-off.** `.claude/hardening-baseline.txt` was raised
    `smelt-cli println 174 → 175` in phase 5 with no sign-off line in the file. Criterion 9 asks
    for one: add a dated comment naming the single legitimate user-facing `--json` stdout call.
11. **Flip phase 8 to `done`** in `outcome.md`'s phase table and write `08-summary.md`.

## The nine-criteria verification table

| # | How it is checked | Where the evidence lives |
|---|---|---|
| 1 | `PropertyProfile` field list read against §"The property profile"; run the gate | `crates/smelt-logical/src/analysis/profile.rs`; `cargo test -p smelt-cli --test property_profile_parity` (3 tests) |
| 2 | `diff_purity` asserts no I/O; exhaustiveness is structural (no `_` arm in `ChangeKind::direction`, no `..` in the `PropertySet` destructure) | `crates/smelt-logical/tests/diff_purity.rs`; `cargo test -p smelt-logical --lib analysis::diff` (51+) |
| 3 | Baseline module in `smelt-core`; repo-state test; exit-2 mapping. **Wording corrected in phase 4**: the profile is assembled by `smelt-runtime`'s `build_model_diagnostics`, not a "thin `smelt-db` profile query" — `smelt-db` cannot depend on `smelt-runtime`. `outcome.md` criterion 3 already records the correction | `cargo test -p smelt-core --test baseline` (20), incl. `diff_leaves_no_repository_state`; `smelt_cli::errors::exit_code_for_baseline_error_is_2` |
| 4 | Flags + exclusivity + both fixtures. **Corrected in phase 5**: `SUM → MAX` shifts nothing in `examples/timeseries`; the fixture uses the `raw.users` join and `user_spend_running_total`. `outcome.md` criterion 4 and the Decision log already record it | `cargo test -p smelt-cli --test property_diff_cli` (16), incl. `a_join_induced_downgrade_propagates_to_the_named_downstream_model` and `losing_incremental_maintenance_reports_a_maintenance_lost_downgrade` |
| 5 | JSON key set asserted against §"Output forms"; flags documented | `property_diff_cli.rs` schema test; `cargo test -p smelt-cli --test cli_docs_coverage` |
| 6 | Marker literal identical in code/docs/workflow; `<details open>` rule; workflow present | `cargo test -p smelt-cli --test property_diff_ci_docs` (3); `.github/workflows/property-diff.yml`; `docs-site/docs/guide/ci.md` |
| 7 | Lens/diagnostic parity with the CLI JSON, proven non-vacuous by phase 7's sabotage run | `cargo test -p smelt-lsp --test property_diff_parity`, `--test property_diff_refresh`, `--test property_diff_coalescing`, `--test property_diff_overlay` |
| 8 | This phase. Tasks 2–5, 7, 8 land it; task 9 is the check | `docs/validations/2026-09-06-property_diff.md`; ROADMAP diff |
| 9 | Staged gate run (below); baseline sign-off from task 10 | phase summary gate block |

## Risks

- **`/smelt:validate` recommends a code fix, not a doc fix.** If the drift report names a
  semantics divergence rather than a doc one, do **not** change behaviour in this phase —
  record it as a §Known Divergence and say so plainly in the summary. A closure phase that
  quietly ships behaviour is worse than one that closes with a named gap.
- **Task 6 rabbit-holes.** Time-box it. The adjudication ("harness or transport, with evidence")
  is the deliverable; a fix is not.
- **ROADMAP wording drifts into plan vocabulary.** ROADMAP is exempt from the timeless-oracle
  rule (it *is* history), but the spec and `docs-site/` are not — keep the phases in the ROADMAP
  entry only.

## Verification gate

Staged, never as one `verify-phase.sh` call (120s tool timeout; pass explicit `timeout`, max
600000ms). Another session may be building — keep `CARGO_BUILD_JOBS=4 --test-threads=4`:

```
cargo fmt --all -- --check
bash .claude/scripts/clippy-gate.sh 2>&1 | tail -40
cargo test -p smelt-db --test integration diagnostics_catalogue --quiet
cargo test -p smelt-cli --test property_profile_parity --test property_diff_cli \
    --test property_diff_ci_docs --test cli_docs_coverage --quiet
cargo test -p smelt-logical --test diff_purity --test walk_coverage --quiet
cargo test -p smelt-core --test baseline --test hardening_budget --quiet
cargo test -p smelt-runtime --test execute_parity --test profile_workspace --quiet
cargo test -p smelt-lsp --test property_diff_parity --test example_workspaces --quiet
cargo test -p smelt-cli --test example_diagnostics --quiet
CARGO_BUILD_JOBS=4 cargo test --workspace --quiet -- --test-threads=4 2>&1 | tail -40
cd docs-site && mkdocs build --strict
```

Only tasks 2, 5, 6 can plausibly move a test; the rest are doc-only and the full run is the
criterion-9 evidence.

## Commit message

```
phase(property-diff/8): docs sweep and closure — clear divergences, fix References, ROADMAP

Deletes the stale "specified and unimplemented" bullet for PropertyDowngrade /
PropertyDiffBaselineUnavailable, rewrites property_diff.md §References onto the paths that
exist (the feature was driven from docs/outcomes/, not a plan file), corrects the Overview
example and editor-features.md's open-buffer claim, records the divergences that lived only in
phase summaries, and lands the ROADMAP completion entry. /smelt:validate property_diff: zero
drift (docs/validations/2026-09-06-property_diff.md).
```
