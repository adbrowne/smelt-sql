# Phase 6b summary — Deferral frontier for the succession grain

## Shipped

- `crates/smelt-runtime/src/maintenance_driver/succession/frontier.rs`:
  `record_succession_frontiers` (mirrors the ordinary incremental path's
  interval-store `get_or_create`/`record_interval` and per-source
  `record_landing` blocks, lifting the posture mapping rather than
  re-deriving it) and `build_succession_run_record` (the succession
  dispatch's `ModelRunRecord` constructor, moved out of
  `execute/project/mod.rs` to pay for the new call site and keep that file
  at its large-file baseline, 4689).
- `execute/project/mod.rs`'s succession window-forward branch now calls
  `record_succession_frontiers` after `execute_succession_maintenance`
  succeeds and before the manifest insert; the rebuild branch is untouched
  (no run window to record).
- Spec delta landed in `docs/specs/incremental_shapes.md` §"Run shape and
  late events": one sentence stating a succession run records its own
  interval-ledger window and source landing on the same terms as every
  other maintained grain.
- Tests: `crates/smelt-runtime/tests/succession_frontiers.rs` (3 new tests:
  maintained-interval recording, source-landing recording, rebuild records
  neither), a 4th end-to-end test
  `succession_deferral_skip_is_licensed_end_to_end` added to
  `contract_deferral_skip_e2e.rs`, and phase 7d's tests 6–7 re-added
  verbatim (by intent, adapted to the succession recipe API) into
  `crates/smelt-cli/tests/maintenance_conformance/contract_points.rs` as
  `succession_deferral_recipe_upholds_restated_oracle_with_a_skipped_run`
  and `succession_deferral_leg_is_not_vacuous`.

## Decisions

- 2026-09-07: the shared fixture `tests/fixtures/succession/smelt.yml`
  declares no `state:` block (defaults to `StateMode::Stateless`), which is
  fine for `succession_patch_e2e.rs` (never reads `.smelt/`) but silently
  no-ops every `.smelt/` write these new tests need to observe. Rather than
  editing the shared fixture (risking other tests' behaviour), the new
  `succession_frontiers.rs` harness appends `state:\n  mode: intervals\n` to
  its own copied `smelt.yml` after `copy_dir_recursive`.
- 2026-09-07: per task 7, per-cell (`contract.cells[].deferral`) frontier
  advancement (`advance_cell_frontiers`) was deliberately NOT wired for the
  succession grain — this phase writes the model-level frontier only. A
  succession model derives exactly one cell (the whole `SuccessionPatch`
  cell), so there is no per-cell frontier today that would differ from the
  model-level one; a future phase should revisit this only if the grain
  ever grows more than one derived cell.
- The plan's Verification section names `--test succession_patch_e2e`; that
  file is a submodule of the `technique_lowering` test binary (declared via
  `tests/technique_lowering/main.rs`), not its own cargo test target — ran
  `cargo test -p smelt-runtime --test technique_lowering` instead (37
  passed, including both `succession_patch_e2e` tests).

## For the next planner

- Criterion 6's contract-lattice `deferral` leg (phase 7d's original scope)
  is now fully closed: recipe-level `contract:` field + admission tests
  (7d) plus the executed-skip conformance legs (this phase). No further
  succession/deferral work is outstanding from that criterion.
- Not investigated: whether a genuinely late append landing in an
  already-processed succession window would now cause `resolve_
  deferral_frontiers` to see a maintained frontier that's briefly "ahead"
  of a re-presented window's actual fold state — the frontier write happens
  once per completed window-forward step, same granularity as the ordinary
  path, so this is presumed fine by the same reasoning that path already
  relies on, but was not specifically re-verified for the succession
  driver's re-run-tolerant semantics.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both
  feature sets, full workspace `cargo test`, `example_diagnostics`)
- `cargo test -p smelt-runtime --test succession_frontiers --test
  contract_deferral_skip_e2e` — 3 + 6 passed
- `cargo test -p smelt-runtime --test technique_lowering --test
  statement_parity --test execute_parity` — 37 + 41 + (execute_parity ran
  clean via workspace suite) passed
- `cargo test -p smelt-cli --test maintenance_conformance --quiet` — 99
  passed (full seeded sample, up from 97 before this phase)
- `bash .claude/scripts/large-file-check.sh` — OK
