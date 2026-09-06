# Outcome: Property diff — "explain the diff" for model edits

**Created:** 2026-09-05
**Status:** active — all 8 phases done, but criterion 7's promised lens *action* ("Executing
the lens opens the text report… in the editor's output channel") is not implemented in any
editor; see `docs/outcomes/20260905-property-diff/phases/08-summary.md` for the honest
nine-criteria reading. Left as `active` rather than `done` for the controller to rule on
(ruling R6).
**Source:** `docs/specs/property_diff.md` (spec commit `5b439b80`); `docs/research/20260905-ten-directions.md` item 4
**Spec anchors:** `docs/specs/property_diff.md` §Surface (`smelt explain --diff`, "Output forms", "Pull-request comment", "Editor", "Diagnostics"), §Semantics ("The property profile", "The diff", "Direction", "Attribution", "Baseline materialisation"), §Constraints items 1–9; `docs/specs/architecture.md` §"Salsa purity rule (analysis)", §"Workspace loading parity rule (CLI ↔ LSP)", §"Project isolation rule"

## The outcome

Editing a model's SQL, its `smelt.yml` override, or a source declaration shows the modeller what
the edit did to smelt's proofs before anything runs. A per-model **property profile** — grain, row
identity, per-source bound/reach, per-cell technique and contract point, refusals, probe set — is
one pure value single-owned in `smelt-logical`, and the existing `smelt explain <model>` report
is a rendering of it. `smelt explain --diff [<ref>]` derives the profile of every model at a git
baseline (default: merge-base with `main`, materialised by `git archive` and loaded through
`load_workspace`) and at the working tree, diffs them with one direction table, attributes each
shifted model to its nearest edited ancestors, and prints text, JSON, or a Markdown body a CI
job posts as one pull-request comment. The editor shows a code lens on every shifted model and a
`PropertyDowngrade` warning on every downgrade, from the same diff value the CLI produces.

## Success criteria (checkable)

1. `smelt_logical::analysis::profile::PropertyProfile` exists with exactly the fields
   `property_diff.md` §"The property profile" lists (`properties: PropertySet`, `cells`,
   `refusals`, `probes`); `smelt_runtime::diagnostics::ModelDiagnostics`
   and the CLI report render from it. Standing gate
   `cargo test -p smelt-cli --test property_profile_parity`: for every maintained model in
   `examples/timeseries` and `examples/retail_analytics`, the report's JSON for grain, bounds,
   cells, refusals, contract point, and probes is byte-identical to the profile's encoding.
2. `diff_profiles(old, new, graph)` is a pure function in `smelt-logical` (no I/O, no ledger,
   no backend); every `Dimension` has one direction rule in one table and a missing rule is a
   compile error (exhaustive match, no wildcard). Unit tests cover each row of the §"Direction"
   table, the added/removed/unshifted cases, rename-as-removal-plus-addition, and attribution
   to nearest edited ancestors including the `of: []` project-config case.
3. Baseline materialisation lives in `smelt-core`, shells out to `git` (`rev-parse`,
   `merge-base`, `archive`), loads through `load_workspace`, and leaves no worktree, stash,
   index entry, ref, or scratch directory behind (a test asserts `git status --porcelain` and
   `git worktree list` are unchanged after a diff). An unresolvable baseline is
   `PropertyDiffBaselineUnavailable` / exit `2`, never an empty diff. The profile for each side is
   assembled by the single-owner `smelt-runtime` builder (`build_model_diagnostics`), consumed by
   a thin `smelt-db` query (`maintenance_plan_report`) over pure `smelt-logical` functions — not a
   "thin `smelt-db` profile query" (`smelt-db` cannot depend on `smelt-runtime`, which owns the
   probe plan a profile carries; corrected per `docs/specs/property_diff.md` §Interactions
   "Salsa purity").
4. `smelt explain --diff [<ref>]` with `--json`, `--markdown`, `--fail-on {downgrade,any}`,
   `--select` (reported set only) works as specified; combining `--diff` with `<model>`,
   `--show-sql`, `--period`, or `--technique` exits `2`. A real-fixture test copies
   `examples/timeseries` into a temp git repo, commits, and edits `user_daily_spend` to add a
   join to the unclocked `raw.users` dimension — breaking its row identity — which asserts the
   edited model shows a `cell_technique` downgrade and its downstream `user_spend_running_total`
   shows cause `downstream` with `of: [user_daily_spend]`; a formatting-only edit yields
   `no models shifted`. (A plain `SUM` → `MAX` combiner swap, tried first, produces no shift at
   all here — see the Decision log: invertibility only matters for a correction/`UpstreamMutation`
   cell, and `user_daily_spend`'s only combiner-sensitive cell is a `NewData` fold over an
   append-only source, which never needs one.) A second fixture flips `user_daily_spend`'s
   `refresh: incremental` to `refresh: full` and asserts the resulting `maintenance_lost`
   downgrade trips `--fail-on downgrade` to exit `1` — the headline "losing incremental
   maintenance is silent" case, exercised end to end.
5. The JSON schema in §"Output forms" is emitted exactly; `old`/`new` values reuse the
   single-version report's encodings; `cli_docs_coverage` covers the new flags.
6. The Markdown form carries the `<!-- smelt-property-diff -->` marker, one `<details>` per
   shifted model open when it holds a downgrade; `docs-site/docs/guide/ci.md` documents the
   GitHub Actions job (`smelt explain --diff "$BASE_SHA" --markdown` → `gh pr comment`, update
   not stack), and this repository's PR workflow runs that job over `examples/` as dogfood.
7. The LSP advertises code lens; a shifted model file gets one lens
   `N downgrades, M upgrades vs <short ref>` and one `PropertyDowngrade` warning per downgrade
   at the specified anchor; an unshifted model gets neither; a non-git workspace gets neither
   and logs at `info`. The baseline is cached per resolved commit and re-resolved when
   `.git/HEAD` or the ref it names changes. Standing gate
   `cargo test -p smelt-lsp --test property_diff_parity`: lens counts and the
   `PropertyDowngrade` set equal `smelt explain --diff --json` for the same workspace and ref.
8. `PropertyDowngrade` and `PropertyDiffBaselineUnavailable` are in `DiagnosticCode`, the
   `docs/specs/diagnostics.md` catalogue, and `docs-site/docs/reference/diagnostics.md`;
   `docs-site/docs/reference/smelt-explain.md` and `docs-site/docs/guide/editor-features.md`
   describe the surface; `docs/specs/property_diff.md` §Known Divergences no longer says the
   feature is unimplemented; ROADMAP records completion. `/smelt:validate property_diff` reports
   zero drift.
9. Standing gates green: `verify-phase.sh`, `example_diagnostics`, `example_workspaces` (LSP),
   `execute_parity`, `hardening_budget` (new `unwrap`/`expect` classified, baseline updated only
   with a sign-off line), `diagnostics_catalogue`, `cli_docs_coverage`.

## Out of scope

- **Deployed-snapshot baseline** (`--against deployed`) — §Future Extensions; the migration plan
  in `definition_deltas.md` already answers the warehouse-side question.
- **Two-ref diffs** and **rename detection** — §Limitations, decided against by the spec.
- **Cost estimates on downgrades** — depends on the cost-aware planner direction, not started.
- **An in-memory overlay loader** for the baseline — rejected in §Design; the loader stays
  filesystem-based and parity-rule untouched.
- **Multi-project CLI invocation** — one `--project-dir` per call; the CI guide shows the loop.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Diagnostics catalogue + spec cross-refs: add `PropertyDowngrade` and `PropertyDiffBaselineUnavailable` to `DiagnosticCode`, `diagnostics.md`, `map_metadata_error_to_diagnostic` exhaustiveness untouched; `cli.md`/`lsp.md` pointers to `property_diff.md` | done |
| 2 | `PropertyProfile` in `smelt-logical` derived by the same pure functions the report uses; `ModelDiagnostics` and the CLI report render from it; `property_profile_parity` gate over both example workspaces | done |
| 3 | `diff_profiles`: dimensions, the exhaustive direction table, matching rules, added/removed/unshifted, attribution over the dependency graph; unit tests per §"Direction" row | done |
| 4 | Baseline materialisation in `smelt-core`: ref/merge-base resolution, `git archive` into scratch, `load_workspace`, cleanup guarantee, `PropertyDiffBaselineUnavailable`; `smelt-runtime::profile::profiles_for_workspace` assembling both sides' profile maps | done |
| 5 | `smelt explain --diff` text + JSON + `--fail-on` + `--select` + flag exclusivity; temp-git-repo fixture test over `examples/timeseries`; `smelt-explain.md` reference page | done |
| 6 | `--markdown` renderer with marker and `<details>`; `docs-site/docs/guide/ci.md`; dogfood job in `.github/workflows` posting/updating one comment over `examples/` | done |
| 7 | LSP: code lens capability, baseline cache keyed on resolved commit with `.git/HEAD` watch, `PropertyDowngrade` anchoring, non-git silence; `property_diff_parity` gate; `editor-features.md` | done |
| 8 | Docs sweep and closure: `diagnostics.md` user page, spec Known Divergences cleared, ROADMAP entry, `/smelt:validate property_diff` zero drift | done |

## Decision log

<!-- Dated entries: decision, evidence, how to reverse. -->

**2026-09-05 — a combiner swap (`SUM` → `MAX`) is a downgrade only where invertibility is
needed, never over a `NewData` fold on an append-only source.** Phase 5's original criterion-4
fixture plan assumed editing `user_daily_spend`'s `SUM(amount)` to `MAX(amount)` would downgrade
its `cell_technique` and propagate to a downstream mart. Verified by hand against
`examples/timeseries`: it does not — not even a discriminant-level shift beyond a neutral
metadata note. Root cause: `user_daily_spend`'s only combiner-sensitive cell is a `NewData` fold
over the append-only `raw.transactions` source. A `NewData` fold only ever *adds* new rows to the
running aggregate; it never needs to *retract* a previously-folded value, so it never needs the
combiner to be invertible. `MAX`, `AVG`, and even `SUM(DISTINCT …)` all stayed admitted as
`KeyedFold`, and even a genuinely holistic combiner (`MEDIAN`) that loses the cell entirely still
produced zero downstream propagation, because `user_daily_spend`'s downstream consumers derive
their own admission from their own SQL, not from whether the upstream still has a maintained
cell. Invertibility only becomes load-bearing for a correction cell (`UpstreamMutation` trigger,
needed when the driving source can retroactively change an already-folded row) — none of
`examples/timeseries`'s SUM-aggregating models read from a source declared mutable. The
criterion-4 fixture instead uses an edit that breaks `user_daily_spend`'s row identity (a join to
the unclocked `raw.users` dimension), which does downgrade its `cell_technique` and does
propagate to `user_spend_running_total`. Reversal: none needed — this is a fact about the
feature's sensitivity, not a workaround; a future fixture wanting to demonstrate a pure
combiner-driven downgrade needs a model whose driving source is declared mutable (not
append-only).

**2026-09-06 — a non-partition-local `cell_added` and a widened grain are downgrades; a new
cell never grades `upgrade`.** Dogfooding the PR comment on #191/#192 showed the first direction
table graded every new cell an upgrade and a widened row key an upgrade. Both are wrong for the
reviewer: a new `JOIN` to an unclocked source reads it in full every run and rebuilds the model
whenever it changes, and a wider key is a weaker uniqueness claim. The direction table now grades
those downgrades, reserves the upgrade for `maintenance_gained`, and grades a cell removed because
its source was dropped `neutral`. Rationale lives in `docs/specs/property_diff.md` §Design "A new
dependency is a cost, not an upgrade" and the grain paragraph under §"Direction"; landed via
`docs/plans/20260906-property-diff-stories.md`, which also replaced the verdict-table-first
rendering with severity-ranked stories (§"Stories"). Reversal: edit the two rows in
`ChangeKind::direction` and the spec table together; the `story_coverage` gate and the
`property_diff_cli` real-fixture tests pin the current grading.

## Blocked

<!-- Dated entries; each names the phase, what blocked it, and what a human must decide. -->
