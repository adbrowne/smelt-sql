# Phase 7 summary — LSP code lens, PropertyDowngrade, baseline cache, parity gate

**Shipped.** `smelt-lsp` gained `smelt-runtime`/`smelt-logical` deps (python feature forwarded).
Pipeline extracted to `smelt_runtime::property_diff::{work_side, baseline_side, report}`; the CLI
(`explain_diff.rs`) now calls it with no behaviour change (`property_diff_cli` 16/16 green — one
fix needed: `anyhow::Error::downcast_ref` only inspects the type `.context()` wrapped, not its
transitive `source()` chain, so `Baseline` errors must convert to `anyhow` un-wrapped to keep the
exit-`2` classification working). LSP side: `crates/smelt-lsp/src/property_diff.rs`
(`ProjectDiffState`, `anchor_for`, `diagnostics_for_model`, `refresh`), wired into `backend.rs`:
`codeLens` capability + handler, `PropertyDowngrade` merged into the existing
`publish_diagnostics` call (never a second publish), `refresh_property_diff` in `spawn_blocking`
triggered on `initialized`/`did_save`/external `.sql` change/`.git` HEAD-refs-packed-refs change,
coalesced via a `running` flag, baseline cached per `(project_root, commit)` and reused until
re-resolution finds a different commit. Spec Δ1–Δ4 landed in `property_diff.md` §Surface "Editor".

**Gotcha found and documented** (`crates/smelt-lsp/CLAUDE.md`): the property-diff refresh trigger
was originally placed after `initialized`'s `client.register_capability(...).await` — which hangs
forever in a test harness that never answers server-initiated requests, silently starving
everything after it. Moved before that call, matching `publish_source_diagnostics`'s existing
placement rationale.

**DuckDB check (R1):** built `smelt-lsp` binary, ran `ldd target/debug/smelt-lsp` — no `libduckdb`
in the link list (only python/libc/libz/libexpat). `cargo tree -p smelt-lsp -i duckdb` finds no
`duckdb` package at all in the resolved graph. Confirmed safe per the ruling.

**example_workspaces / example_diagnostics (R4):** `examples/` has zero diff vs `main` on this
branch right now (`git diff $(git merge-base HEAD main) HEAD -- examples/` is empty), so the
collision hasn't materialized yet — but the filter was added defensively anyway, narrow to
`property-downgrade` only, with a comment explaining why (advisory, not validity). Both gates run
green: `example_workspaces` 35/35, `example_diagnostics` 121/122 (1 ignored, pre-existing).
`example_diagnostics` needed no change — it queries Salsa directly and never touches the LSP's
property-diff state.

**Sabotage run (R3, required):** in `derive_state_maps`, changed
`let diags = diagnostics_for_model(model, &ast);` to `let mut diags = ...; diags.pop();` (drop the
last downgrade). Reran `property_diff_parity`: failed with `assertion left == right: ... left: 3
right: 4` on the PropertyDowngrade-count assertion for `user_daily_spend`. Reverted; gate green
again (0.97–1.95s). Confirms the gate is not vacuous.

**Tests and what breaks them:**
- `anchor_column_subject_hits_the_select_item`, `anchor_cell_subject_falls_back_to_first_sql_token`,
  plus two more anchor unit tests: fail if anchoring stops matching aliases or always falls back.
- `short_ref_abbreviates_a_sha_and_leaves_a_named_ref_alone`, `lens_title_matches_the_summary_counts`
  (smelt-logical): fail on a wrong truncation rule or a downgrade/upgrade count swap.
- `property_diff_parity` (e2e, real LSP over duplex streams): fails if the lens/diagnostic
  pipeline breaks anywhere from project routing through anchoring — proven by the sabotage run.
- `non_git_workspace_is_silent`, `baseline_is_reused_until_head_moves` (direct on `refresh`): the
  latter fails against no-cache (leg 1) or a project-root-only cache (leg 3), via `Arc::ptr_eq`.
- Cannot independently verify: the VS Code `smelt.showPropertyDiff` command (not built, see
  below) and the `.git` watcher's *registration* (glob construction is tested only via
  `derive_git_watch_globs` unit coverage, not a real client round-trip).

**Deviations:**
- Task 10 (VSCode command) skipped — recorded as a spec Known Divergence in behavioural terms
  (executing the lens is currently a no-op in every editor).
- Baseline-resolution failures are all treated as `Silent` uniformly (not narrowly scoped to the
  three "not a git tree"-class variants) — recorded as a spec Known Divergence.
- `mkdocs build --strict` not run (not in this phase's stated gate); `editor-features.md` reviewed
  by hand instead.

**Gate results:** `cargo fmt --all -- --check` clean; `cargo check --workspace --all-targets`
clean; `clippy-gate.sh` clean on both feature sets; `smelt-lsp` full suite green (20+35+2+3+6+129
+6+2+5+3+11+1+2+3+12 across files); `hardening_budget` unaffected (no new unwrap/expect in
production code — new ones are test-only). Commit range: this phase's commits on
`outcome-20260905-property-diff`.

**For Phase 8:** docs sweep — verify `docs-site/docs/reference/smelt-explain.md` still matches the
CLI's current output (no change expected, but re-check now diff pipeline moved crates); confirm
`docs/specs/property_diff.md` §References' code/test paths are accurate (updated this phase);
decide whether to build the VSCode `smelt.showPropertyDiff` command or formally close it out as a
divergence; sweep for any remaining `Phase [A-Z0-9]` timeless-oracle violations across the whole
feature's docs before final closure.
