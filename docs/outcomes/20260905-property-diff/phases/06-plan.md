# Phase 6 plan — `--markdown`, `docs-site/docs/guide/ci.md`, dogfood workflow

**Outcome:** `docs/outcomes/20260905-property-diff/outcome.md`, criterion 6.
**Spec:** `docs/specs/property_diff.md` §Surface "Output forms" (Markdown), §"Pull-request
comment", §Constraints item 5 ("Surface parity").
**Carry:** `phase67-carry.md` D1 (measure `examples/huge`), D2 (record the resolver gap in
§Known Divergences), D3 (every test must be able to fail).

## Objective

Add `smelt explain --diff --markdown`: a single GitHub-flavoured comment body carrying the
`<!-- smelt-property-diff -->` marker and one `<details>` per shifted model, open when that model
holds a downgrade. Document the GitHub Actions job that posts/updates one comment in
`docs-site/docs/guide/ci.md`, and run that job over `examples/` on this repository's own pull
requests as dogfood.

## Spec delta (write these into `docs/specs/property_diff.md` FIRST)

The Markdown paragraph and §"Pull-request comment" are underspecified in three places that the
implementation cannot resolve silently.

- **Δ4 — the marker is unconditional.** §Surface "Output forms" → Markdown: the heading and the
  trailing marker are emitted *even when nothing shifted* (body: the same
  `property diff vs <ref>: no models shifted` line). Without this the CI job cannot update its
  previous comment to the cleared state, and a stale "4 downgrades" comment stands after the
  author fixes the regression. Also state the `<summary>` line's shape: `<model name> — <cause>
  — N downgrades, M upgrades, K neutral`, with `<cause>` the same string the text form's block
  header uses.
- **Δ5 — the body is bounded.** GitHub rejects an issue comment over 65,536 characters. Specify:
  the Markdown form renders at most the first 50 model blocks in full; any remainder is listed by
  name only inside one final `<details>` (`… and N more shifted models`), and the marker is always
  the last line. The cap is on the *rendered* body only — it never changes `summary`, `--fail-on`,
  or the JSON form.
- **Δ6 — the documented job on a fork PR.** §"Pull-request comment": a `pull_request` event from a
  fork gets a read-only `GITHUB_TOKEN`, so the documented job always writes the body to the job
  summary and posts/updates the comment only when the head repository is the base repository.
  State also that the job does **not** gate the build by default: `--fail-on` is offered as an
  opt-in for user projects, and this repository's dogfood job runs without it.

D2 lands here too, since Δ4–Δ6 already open the spec: add a §Known Divergences bullet, in
behavioural terms, that a non-model `.sql` file with no `smelt.` marker (a DDL/setup script)
classifies as a model and fails profile derivation; today it is symmetric on both sides and never
reaches user-facing output, but an edit to such a file on one side only would surface as a
spurious `added`/`removed` entry carrying the derivation-failure text as its reason.

`docs/specs/property_diff.md` §Known Divergences bullet 1 ("not yet implemented") stays until
Phase 8.

## Design decisions

**D6.1 — the renderer lives in `smelt_logical::analysis::diff_render`, beside `text_report`.**
New public items: `pub const MARKER: &str = "<!-- smelt-property-diff -->"` and
`pub fn markdown_report(report: &DiffReport) -> String`. It reads `report.models` in the order
`diff_profiles` produced and never re-sorts, exactly as `text_report` does.

**D6.2 — reuse is at the field-primitive level, and it is asserted, not asserted-by-comment.**
The spec mandates the Markdown table carry "the same columns as the JSON `changes` entries", so a
row is not literally `change_line`'s string — but every *value* in it must be the one the text form
prints. Therefore promote `diff_render`'s currently-private `glyph`, `dimension_str`,
`json_display` and `cause_str` from `fn` to `pub fn` (documented as the shared primitives of every
rendered form) and have `markdown_report` call them. `change_line` itself is reused verbatim for
the `<summary>`-adjacent per-model text and by Phase 7. No new dimension spelling, glyph, cause
string, or `old`/`new` encoding is written in the Markdown path. The parity test in the TDD list
(`markdown_values_match_the_text_form`) is what actually enforces this; the doc comment is not.

**D6.3 — `<details open>` is computed from the model's own change directions**, i.e.
`model.changes.iter().any(|c| c.direction == Direction::Downgrade)` — not from `report.summary`,
which is a whole-report count and would open every block whenever any model downgraded.

**D6.4 — the CLI flag.** `--markdown` on `ExplainArgs` with
`#[arg(long = "markdown", requires = "diff", conflicts_with = "json")]`, matching Phase 5's
`--fail-on` (`requires = "diff"`) and `--diff` (`conflicts_with_all`) pattern. Clap produces exit
`2` for both violations through the existing usage-error path. In `explain_diff.rs` the render
branch becomes a three-way `if args.json { … } else if args.markdown { print!("{}",
markdown_report(&report)) } else { text_report }`, placed *before* the `--fail-on` block so the
body is always emitted even when the exit code is `1`.

**D6.5 — how the workflow finds and updates its comment.** `gh pr comment --edit-last` edits the
last comment by the actor regardless of content, which would clobber an unrelated bot comment.
Use the marker instead:

```bash
id=$(gh api "repos/$GITHUB_REPOSITORY/issues/$PR/comments" --paginate \
      --jq '[.[] | select(.user.login=="github-actions[bot]")
                 | select(.body | contains("<!-- smelt-property-diff -->"))] | last | .id // empty')
if [ -n "$id" ]; then
  gh api -X PATCH "repos/$GITHUB_REPOSITORY/issues/comments/$id" -F body=@body.md
else
  gh pr comment "$PR" --body-file body.md
fi
```

First run: `id` is empty, so the `gh pr comment` branch creates it. Every later push takes the
PATCH branch — one comment, updated, never stacked. `-F body=@file` is `gh api`'s file-valued
field form; `-f` would send the literal string.

**D6.6 — permissions and forks.** The job needs `permissions: { contents: read, pull-requests:
write }`. On a `pull_request` event from a fork the token is read-only and both `gh` calls 403.
Rather than let that fail confusingly, the job always appends the body to `$GITHUB_STEP_SUMMARY`
(which works for forks), and guards only the comment step with
`if: github.event.pull_request.head.repo.full_name == github.repository` — the same guard
`docs-pr-preview.yml` already uses for the same reason. The guide states the `pull_request_target`
alternative and warns against it (it grants a write token to a workflow that must not check out
untrusted head code).

**D6.7 — where the dogfood job goes.** A new `.github/workflows/property-diff.yml`, not a step
inside `test.yml`: it needs `pull-requests: write` (`test.yml` has no `permissions:` block and
should not gain one workspace-wide), it runs only on `pull_request`, and it must not be a
dependency of the test matrix. It needs no warehouse — `explain --diff` never connects to a
backend — so it is a `cargo build -p smelt-cli --no-default-features --features duckdb` plus two
CLI invocations. It requires `fetch-depth: 0` so `git merge-base` can resolve, and it passes the
PR base SHA explicitly (`smelt explain --diff "$BASE_SHA"`) rather than relying on the merge-base
default, because the actions/checkout merge commit's `main` is not the PR base.

**D6.8 — what the dogfood job asserts: comment only, no `--fail-on`. (recommended)** This
repository's `examples/` shift routinely as maintenance work lands; `--fail-on downgrade` here
would go red on legitimate changes, and a gate that is routinely red is a gate people learn to
bypass — which would also make the *real* signal (an unintended downgrade) invisible. The job's
build-failing assertion is narrower and real: `smelt explain --diff --markdown` must exit `0`
(any non-zero exit, including a panic, an unresolvable baseline, or a profile-derivation failure
in `examples/`, fails the job), and the comment must post. So a broken `--markdown` renderer or a
regression that makes an example project underivable *does* break CI; a legitimate property shift
does not. `docs-site/docs/guide/ci.md` documents `--fail-on downgrade` as the opt-in for user
projects that want the gate, with this tradeoff stated in one sentence.

**D6.9 — what is and is not testable.** Testable by `cargo test`: the Markdown body's whole shape
(marker, `<details>`/`<details open>`, one block per model, table columns, ordering, the Δ5 cap),
the flag exclusivity and exit codes, and — via a text gate over the two files — that the marker
literal in `ci.md` and in `property-diff.yml` is byte-identical to `diff_render::MARKER`. **Not**
testable by cargo, and not to be counted as covered: that the YAML parses, that the token has the
permission, that the `gh api`/`gh pr comment` pair actually finds and updates the previous comment,
and the fork guard. Those are verified once by running the workflow — the summary must record the
observation "PR shows exactly one comment; a second push updated it in place" as a manual check,
not as a passing test.

## TDD test list (red before green; each entry says how it fails against a broken impl)

Unit — `crates/smelt-logical/src/analysis/diff_render.rs` `mod tests`:

1. `markdown_report_of_an_empty_diff_still_carries_the_marker` — an empty `DiffReport` renders the
   heading, the `no models shifted` line, and ends with `MARKER`. *Fails against* an
   implementation that early-returns the text form's bare line (Δ4's whole point) — no marker, the
   workflow can never update a stale comment.
2. `a_model_with_a_downgrade_renders_details_open` — one `ModelDiff` with a `Direction::Downgrade`
   change renders `<details open>`; the same model with only `Direction::Neutral` changes renders
   `<details>` with no `open`. *Fails against* the two obvious wrong impls: always-open, and
   opening on `report.summary.downgrades > 0` (D6.3) — the second is caught by a two-model case in
   the same test where model A downgrades and model B does not, asserting exactly one `<details
   open>` in the body.
3. `markdown_values_match_the_text_form` (surface parity, §Constraints item 5) — for a report with
   one change of each `Direction`, every `dimension_str`, `subject`, `json_display(old)`,
   `json_display(new)` and `cause_str` string present in `markdown_report`'s output is also present
   in `text_report`'s. *Fails against* a Markdown path that spells a dimension itself (e.g.
   `"Cell Technique"`) or formats `old`/`new` with its own `Debug`/quoting.
4. `markdown_preserves_diff_profiles_ordering` — mirrors the existing
   `text_report_preserves_diff_profiles_ordering`: `staging.orders` before `aaa.first`. *Fails
   against* a renderer that sorts by name or by downgrade count.
5. `markdown_table_columns_match_the_json_change_keys` — the table's header row names exactly
   `dimension | subject | direction | old | new | reason`. *Fails against* a table that drops
   `reason` (which the text form emits on its own line and is easy to forget) or renames a column.
6. `markdown_body_of_a_large_diff_stays_under_the_comment_limit` (Δ5) — 500 synthetic shifted
   models render to `< 65_536` bytes, the body contains `and 450 more shifted models`, and the last
   line is still `MARKER`. *Fails against* the naive unbounded renderer, which produces ~200 KB and
   a 422 from the API at post time.

CLI integration — `crates/smelt-cli/tests/property_diff_cli.rs` (reuse `stage_timeseries_repo` and
the criterion-4 `raw.users` join edit; per D3 each of these fails against a no-op `--markdown`):

7. `markdown_and_json_together_is_a_usage_error` — exit `2`. *Fails against* a flag declared
   without `conflicts_with = "json"`, which would silently print JSON and ignore `--markdown`.
8. `markdown_without_diff_is_a_usage_error` — `smelt explain --markdown` exits `2`. *Fails against*
   a missing `requires = "diff"` (the exact hole Phase 5's Q4 found on `--fail-on`).
9. `markdown_reports_the_join_downgrade_in_an_open_details` — after the `raw.users` join edit,
   stdout contains `MARKER`, a `<details open>` whose `<summary>` names `user_daily_spend`, and a
   `<details open>` naming `user_spend_running_total`. *Fails against* a renderer whose open-state
   or cause string is wrong, and against a `print!` branch wired after the `--fail-on` early return.
10. `markdown_body_is_printed_even_when_fail_on_exits_1` — `--markdown --fail-on downgrade` on the
    same edit exits `1` **and** stdout carries the full body. *Fails against* D6.4's ordering hazard
    — a body emitted after the `--fail-on` `return Err` never reaches the workflow's `body.md`, so
    the comment would be empty exactly when it matters most.
11. `a_formatting_only_edit_renders_the_cleared_markdown_body` — no shift ⇒ body is heading +
    `no models shifted` + marker. Pairs test 1 through the real binary.

Docs/workflow text gates — new `crates/smelt-cli/tests/property_diff_ci_docs.rs`:

12. `the_marker_literal_is_identical_in_code_docs_and_workflow` — reads
    `docs-site/docs/guide/ci.md` and `.github/workflows/property-diff.yml`, asserts each contains
    `smelt_logical::analysis::diff_render::MARKER` verbatim. *Fails against* a marker renamed in one
    of the three places — the failure mode that silently turns "update" back into "stack".
13. `the_ci_guide_documents_the_update_not_stack_mechanism` — `ci.md` contains `smelt explain
    --diff`, `--markdown`, `gh pr comment`, and a `PATCH .../issues/comments/` line. *Fails against*
    a guide that documents only the create path (the spec's explicit "update not stack" clause).
14. `the_dogfood_workflow_requests_pull_requests_write` — `property-diff.yml` contains
    `pull-requests: write` and the fork guard string. *Fails against* the job that will 403 on
    every run.

Existing gates that must stay green and will move: `cli_docs_coverage` (add `--markdown` to
`docs-site/docs/reference/cli.md`'s `smelt explain` flag table — it fails until then, which is the
red step for the docs task), `explain_docs_freshness`, `docs_front_door`.

## Tasks

1. Spec Δ4/Δ5/Δ6 + the D2 §Known Divergences bullet in `docs/specs/property_diff.md`. **First.**
2. Tests 1–6 red in `diff_render.rs`; then `MARKER`, `markdown_report`, and the promotion of
   `glyph`/`dimension_str`/`json_display`/`cause_str` to `pub`. Green.
3. Tests 7–11 red in `property_diff_cli.rs`; then `--markdown` on `ExplainArgs` (D6.4) and the
   render branch in `explain_diff.rs` placed before `--fail-on`. Green.
4. `docs-site/docs/reference/smelt-explain.md`: a **Markdown** subsection under §Property diff
   with a short rendered example; `docs-site/docs/reference/cli.md`: `--markdown` row.
5. New `docs-site/docs/guide/ci.md` — the complete copy-pasteable job (D6.5/D6.6), the fork note,
   the `--fail-on` opt-in with the noise tradeoff (D6.8), and the multi-project loop the outcome's
   out-of-scope note points at. Register it in `docs-site/mkdocs.yml` nav under Guide, after
   `Production Deployment` / before `Orchestration`. Timeless-oracle rule: no phase or plan
   vocabulary.
6. New `.github/workflows/property-diff.yml` (D6.7/D6.8): `on: pull_request`, `fetch-depth: 0`,
   mise + `setup-duckdb`, cargo cache keyed `property-diff`, build `smelt-cli`, run
   `--diff "$BASE_SHA" --markdown` over `examples/timeseries` and `examples/retail_analytics`
   concatenated into one `body.md`, append to `$GITHUB_STEP_SUMMARY`, then the guarded
   find-or-create comment step.
7. Tests 12–14 in the new `property_diff_ci_docs.rs`.
8. **D1 measurement.** `time ./target/debug/smelt explain --diff HEAD --project-dir examples/huge`
   (2,002 models, both sides derived, no edits ⇒ the pure profiling cost). Record wall-clock in
   `06-summary.md` with an explicit verdict: acceptable, or pathological and therefore a Phase 7
   blocker for the LSP's per-workspace-load derivation. Do not quietly ship a bad number.

## Risks

- **The dogfood job's first real run is after merge**, so a YAML or `gh` error is only visible then.
  Mitigation: the tests in 12–14 catch the textual mistakes; the shell block must be run once by
  hand against the branch's own PR before Phase 6 is called done, and the observation recorded.
- **`examples/retail_analytics` may hold a model whose profile fails to derive**, which would make
  the dogfood job exit non-zero on every PR (D6.8 deliberately makes that fail the build). Run both
  invocations locally in task 6 before committing the workflow; if one fails, that is a real
  finding to record, not a reason to add `|| true`.
- **`$BASE_SHA` may not be fetched** under a shallow checkout — hence `fetch-depth: 0`. A depth-1
  checkout produces `PropertyDiffBaselineUnavailable`/exit `2`, which looks like a smelt bug.
- **D1 may come back pathological.** That is a finding for Phase 7, not something to fix here.

## Verification gate

Split per the 120 s/10 min trap in `shared-context.md`; never one `verify-phase.sh` call.

```
cargo fmt --all -- --check
bash .claude/scripts/clippy-gate.sh 2>&1 | tail -40
CARGO_BUILD_JOBS=6 cargo test -p smelt-logical --lib 2>&1 | tail -20
CARGO_BUILD_JOBS=6 cargo test -p smelt-cli --test property_diff_cli --test property_diff_ci_docs \
  --test cli_docs_coverage --test explain_docs_freshness --test docs_front_door 2>&1 | tail -40
cd docs-site && uv run mkdocs build --strict 2>&1 | tail -20
```

Plus `bash .claude/scripts/hardening-budget.sh` (a new `println!` in `smelt-cli` for the Markdown
branch bumps the baseline by one — legitimate user-facing stdout, note it in the summary the way
Phase 5 did).

## Commit message

```
feat(property-diff): --markdown output form, CI guide, and dogfood PR comment

`smelt explain --diff --markdown` renders one GitHub comment body: heading,
one <details> per shifted model (open when it holds a downgrade), a change
table with the JSON columns, and the <!-- smelt-property-diff --> marker a
workflow uses to update its previous comment instead of stacking. The body is
bounded so a large diff cannot exceed GitHub's comment limit, and the marker
is emitted even for an empty diff so a stale comment can be cleared.

Rendering reuses diff_render's shared primitives (glyph, dimension_str,
json_display, cause_str) rather than re-deriving any spelling; a parity test
asserts every value in the Markdown body appears in the text form.

Adds docs-site/docs/guide/ci.md documenting the GitHub Actions job, and
.github/workflows/property-diff.yml running it over examples/ on this
repository's own pull requests (comment-only, no --fail-on).

Spec: docs/specs/property_diff.md Δ4-Δ6 + Known Divergences (resolver gap).
```
