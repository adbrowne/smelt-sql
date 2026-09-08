# Phase 9 summary — Close-out

## Outcome: all seven criteria met at HEAD. Status flipped to `done`.

## Criterion → evidence table

| # | Criterion | Evidence | Command | Result |
|---|---|---|---|---|
| 1 | Spec first, timeless-oracle | `docs/specs/sources.md` §"Externally-produced sources"; only "Phase " hit in the diff is the rule's own definition line, not a violation | manual grep `Phase [A-Z0-9]` over changed sections | clean |
| 2 | Declaration + validation, fail-loud | `diagnostics_catalogue` test; `smelt-core` external_step unit tests; `examples/broken/` fixtures | `cargo test -p smelt-db --test integration diagnostics_catalogue`; `cargo test -p smelt-core --test external_step_command --test external_step_graph --test external_step_yaml` | 1 + 25 passed |
| 3 | DAG membership | selector/list coverage | `cargo test -p smelt-cli --test list_external_step` | 3 passed |
| 4 | Invocation + failure/refusal | runtime invocation + reporting suites | `cargo test -p smelt-runtime --test external_step_invocation --test external_step_reporting` | 7 + 6 passed |
| 5 | Explain (text + JSON) | dedicated dispatch test + CLI docs coverage | `cargo test -p smelt-cli --test explain_external_step --test cli_docs_coverage` | 3 + 10 passed |
| 6 | Fixture + docs | example gates, docs freshness (incl. new gate below), mkdocs build | `cargo test -p smelt-cli --test example_diagnostics`; `cargo test -p smelt-lsp --test example_workspaces`; `cargo test -p smelt-cli --test external_step_docs_freshness`; `cd docs-site && uv run mkdocs build --strict` | 126/1 ignored; 37 passed; 9 passed; build clean (one pre-existing INFO: `concepts/incremental-equivalence.md` not in nav — unrelated, see TODO.md) |
| 7 | Gates green, ratchets unmoved | full gate + explicit execute_parity + hardening + large-file | see "Gates" below | ALL GREEN |

## New gate closing phase 8's asymmetry

`external_step_docs_freshness::explain_reference_mentions_external_steps` (red before this
phase — `docs-site/docs/reference/cli.md` never named external steps though `docs/specs/cli.md`
spec's `smelt explain <external step>`). Made green by adding a `### smelt explain <external
step>` subsection to `reference/cli.md`, cross-linked to `guide/external-steps.md`.

## Ratchet audit

`git diff --stat main -- .claude/*baseline*` shows four files moved since `main`, spanning the
whole bigquery-prod branch (dogfood-spine + bigquery-correctness + this outcome). Of the two
`.claude/hardening-baseline.txt` bumps landed by this outcome's own commits
(`b495631d1` phase 5, `4fc84065a` phase 6), both carry sign-off notes in their phase summaries
(`phases/05-summary.md`, `phases/06-summary.md`) — no untraceable movement from this outcome.

## Findings handed back

Appended `## Findings handed back from 20260906-external-dag-steps` to
`docs/handoffs/2026-09-08-github-activity-findings.md`: (a) the shipped loader contract
(`load_day.sh` + `github_loader.yml`, idempotent, invoked by `smelt run`); (b) `smelt list
--format json`'s pre-existing `ListError::ParseErrors` hard-fail on all three example
workspaces; (c) a dry run / `invoke_external_steps: false` refuses rather than reading stale —
the spine's live-run phases will hit this on their first `--dry-run` preview; (d) the UI
plan-preview opt-out named in phase 4's decision log was never built — no call site sets
`invoke_external_steps: false` yet.

## docs/TODO.md additions

Two residues recorded, both confirmed pre-existing and unrelated to this outcome: the `smelt
list --format json` scoping bug, and `concepts/incremental-equivalence.md` missing from the
docs-site nav.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace test, example_diagnostics)
- `cargo test -p smelt-runtime --test execute_parity` — 4 passed
- `cargo test -p smelt-core --test hardening_budget` — 5 passed (includes the gate's own
  self-test injecting and detecting a synthetic regression, not a real one)
- `bash .claude/scripts/large-file-check.sh` — OK
- `cd docs-site && uv run mkdocs build --strict` — clean (pre-existing nav-gap INFO only)

## For the next planner

Nothing further belongs to this outcome — all nine phases are `done`. The findings above are
now the responsibility of `20260906-bigquery-dogfood-spine` (live-BigQuery phases) and
`docs/TODO.md` (the two unrelated residues) respectively.
