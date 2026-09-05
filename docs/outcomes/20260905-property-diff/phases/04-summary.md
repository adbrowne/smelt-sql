# Phase 4 summary — baseline materialisation

No R7 fallback taken — all 10 plan tasks landed.

Commits: `886f0d55` (spec-first: Δ1 profile-assembly ownership, Δ2 frontmatter
in the edited set, `state_downgrade` dimension row), `8544a5dc` (R8:
`CellVerdict.state_downgrade`, `Dimension::StateDowngrade`, diffed and graded),
`320e40f1` (`smelt_core::baseline`: `resolve_baseline`/`materialize`/
`edited_set`, `BaselineError`), `4e049ba4` (D9 step 1: `build_bound_context`
moved to `smelt_runtime::diagnostics`, `pub use` kept in `smelt-cli`),
`8c611af3` (`smelt_runtime::profile::profiles_for_workspace`), `1bc72b85`
(D9 step 2: `property_profile_parity` rewritten onto it; `exit_code_for`
gains the `BaselineError` arm).

## Entry point for Phase 5

`smelt_runtime::profile::profiles_for_workspace(loaded: &LoadedWorkspace) ->
BTreeMap<String, PropertyProfile>` — call it once per side (baseline
`checkout.project_root()`, working tree `--project-dir`), each via
`smelt_core::workspace::load_workspace`, then feed both maps plus a
`DiffGraph` into `smelt_logical::analysis::diff::diff_profiles` (Phase 3).
Git side: `smelt_core::baseline::{resolve_baseline, materialize, edited_set}`.

## Error taxonomy (exit 2)

`smelt_core::baseline::BaselineError`: `NotAGitWorkTree`, `UnknownRef`,
`NoBaseBranch`, `MergeBaseFailed`, `NoProjectAtRef`, `GitUnavailable`,
`Archive`, `Unpack`, `Scratch`. All map to `PropertyDiffBaselineUnavailable`
/ exit `2` via `smelt_cli::errors::exit_code_for`'s new `downcast_ref` arm
(test: `exit_code_for_baseline_error_is_2`). None is recoverable into an
empty diff.

## R10 — baseline-side derivation failure

Not captured here. `profiles_for_workspace` silently omits a model from the
map on any derivation failure (`Err(_) => continue` at the ephemeral-resolver
and `build_model_diagnostics` steps) rather than recording *why* — same
posture as the pre-existing `property_profile_parity` skip. Constraint 6's
"report as added/removed with the derivation failure as its reason" needs
`profiles_for_workspace` to return failure reasons per skipped model, which
it does not yet do. **Left for Phase 5.**

## Cleanup guarantee (R4)

Enforced by `tempfile::TempDir`'s own `Drop`, created before any fallible
step in `materialize` so every error path unwinds through it. The module doc
comment states plainly: `Drop` does not run under `panic = "abort"`,
`std::process::abort`, or SIGKILL — a killed process can leak a scratch dir
under the OS temp dir, which is the backstop, **not** a tested guarantee.
What the tests actually assert: `checkout_scratch_is_deleted_on_drop` (path
gone after a normal drop), `checkout_scratch_is_deleted_when_materialization_fails`
(a bogus commit fails `git archive` after the scratch dir exists; no
`smelt-baseline-*` entry survives), and `diff_leaves_no_repository_state`
(`git status --porcelain`, `worktree list`, `stash list`, `for-each-ref`, and
`.git/index` length+mtime are byte-identical before/after a full
resolve+materialize+load+diff cycle). No test claims anything about a
SIGKILL'd process.

## Other deviations

- Δ2's edited-set predicate compares `ModelMetadata` directly (`PartialEq`
  already derived) rather than via `serde_json::Value` as D7 suggested —
  simpler, same effect.
- `profiles_for_workspace` fixes `MaintenanceDialect::DuckDb` and does not
  wire per-target dialect resolution or live availability resolution
  (`resolve_availability`), so `CellVerdict.state_downgrade` is `None` for
  every cell today — same simplification the pre-existing parity harness
  already made. R8's dimension/grading machinery is real and tested in
  isolation (`state_downgrade_appearing_is_a_downgrade_disappearing_is_an_upgrade`,
  `diff_cell_verdicts_surfaces_a_state_downgrade_on_an_otherwise_unchanged_cell`)
  but has no live producer yet.
- Test 15 (`edited_set_ignores_a_formatting_only_edit`) writes byte-identical
  content rather than a true frontmatter reflow that changes bytes but not
  the stripped form — weaker than the plan's precise ask; still proves the
  "no diff, no edit" base case.

## Gates observed directly (before handing off)

`cargo fmt --all -- --check` clean. `cargo check --workspace --all-targets`
clean. `bash .claude/scripts/clippy-gate.sh` clean on both feature sets.
Per-crate tests all green as run: `smelt-core --test baseline` (20/20),
`smelt-core --test hardening_budget` (4/4, baseline unaffected),
`smelt-logical --lib analysis::diff` (51/51), `smelt-runtime --test
profile_workspace` (2/2), `smelt-runtime --test execute_parity` (4/4),
`smelt-cli --test property_profile_parity --test explain_maintenance`
(36+3/39), `smelt-cli --lib errors` (2/2). I did **not** observe a
`cargo test --workspace` or `example_diagnostics` run complete — I started
`cargo test --workspace --quiet` in the background, it did not finish within
several minutes, and the coordinator is running the remaining full-suite and
`example_diagnostics` gates directly rather than trusting a background
result I cannot receive. Their run is authoritative for those two stages.
