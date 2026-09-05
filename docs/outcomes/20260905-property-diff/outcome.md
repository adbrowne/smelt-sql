# Outcome: Property diff — "explain the diff" for model edits

**Created:** 2026-09-05
**Status:** queued
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
   `PropertyDiffBaselineUnavailable` / exit `2`, never an empty diff.
4. `smelt explain --diff [<ref>]` with `--json`, `--markdown`, `--fail-on {downgrade,any}`,
   `--select` (reported set only) works as specified; combining `--diff` with `<model>`,
   `--show-sql`, `--period`, or `--technique` exits `2`. A real-fixture test copies
   `examples/timeseries` into a temp git repo, commits, edits a staging model (`SUM` → `MAX`),
   and asserts the edited model shows a `cell_technique` downgrade and a downstream mart shows
   cause `downstream` with `of: [<staging model>]`; a formatting-only edit yields
   `no models shifted`.
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
| 4 | Baseline materialisation in `smelt-core`: ref/merge-base resolution, `git archive` into scratch, `load_workspace`, cleanup guarantee, `PropertyDiffBaselineUnavailable`; thin `smelt-db` profile query | pending |
| 5 | `smelt explain --diff` text + JSON + `--fail-on` + `--select` + flag exclusivity; temp-git-repo fixture test over `examples/timeseries`; `smelt-explain.md` reference page | pending |
| 6 | `--markdown` renderer with marker and `<details>`; `docs-site/docs/guide/ci.md`; dogfood job in `.github/workflows` posting/updating one comment over `examples/` | pending |
| 7 | LSP: code lens capability, baseline cache keyed on resolved commit with `.git/HEAD` watch, `PropertyDowngrade` anchoring, non-git silence; `property_diff_parity` gate; `editor-features.md` | pending |
| 8 | Docs sweep and closure: `diagnostics.md` user page, spec Known Divergences cleared, ROADMAP entry, `/smelt:validate property_diff` zero drift | pending |

## Decision log

<!-- Dated entries: decision, evidence, how to reverse. -->

## Blocked

<!-- Dated entries; each names the phase, what blocked it, and what a human must decide. -->
