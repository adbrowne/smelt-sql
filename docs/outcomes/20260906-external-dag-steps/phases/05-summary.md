# Phase 5 summary — Run-path reporting for external steps

**Shipped:**
- `docs/specs/run_state.md` §"Run manifest"/"Run report" and `docs/specs/sources.md` §Semantics 11
  gained the `external_steps` surface and the "success-only, failure aborts before any manifest
  exists" rule.
- `smelt-state::RunManifest.external_steps: BTreeMap<String, ExternalStepRunRecord>` (command,
  produces, duration_ms, outcome), `#[serde(default, skip_serializing_if)]`; mirrored onto
  `RunReport` and copied in `from_manifest` (`crates/smelt-state/src/lib.rs`).
- `RunReporter` gained three defaulted methods: `external_step_started`/`_completed`/`_failed`
  (`crates/smelt-runtime/src/reporter.rs`).
- `invoke_required_steps` (`crates/smelt-runtime/src/execute/external_steps.rs`) now takes
  `&dyn RunReporter` + `run_id`, fires the three events around the spawn, and returns
  `Vec<(String, ExternalStepRunRecord)>` for successes; refusal branches return before any event.
- `execute/project/mod.rs` binds the returned records and folds them into the `RunManifest` literal.
- `CliReporter` implements the three callbacks: a `→ step <addr>` line (argv under `--verbose`), a
  completion line with duration, and a failure line naming the exit code.
- New `crates/smelt-runtime/tests/external_step_reporting.rs` — all 8 planned tests (6 integration +
  2 in `smelt-state`), all passing against a real DuckDB backend.

**Decisions:**
- 2026-09-08: no reshape — the phase's own reshape (splitting old row 5 into rows 5/6) was already
  recorded in the outcome's decision log before this phase ran; this implementation shipped exactly
  what that entry specified.
- Kept `ExternalStepRunRecord.outcome: RunOutcomeKind` (always `Success`) rather than a bare unit,
  per the plan, so it mirrors `ModelRunRecord::outcome` and needs no bespoke serde shape.

**For the next planner:**
- The hardening-budget `println!` ratchet counts `eprintln!` as a substring match — adding
  `CliReporter::external_step_failed`'s `eprintln!` bumped `smelt-cli println` 175→176. Baseline
  updated with a sign-off note in `.claude/hardening-baseline.txt`; worth knowing this ratchet is
  not println!-only if a future phase adds more CLI stderr output.
- Two already-oversized files grew a handful of lines from adding the new field to existing struct
  literals (`crates/smelt-runtime/src/execute/project/mod.rs` 4793→4796,
  `crates/smelt-state/src/file_store.rs` 1690→1695) and `crates/smelt-cli/tests/resume.rs` (1058→1059).
  All three are mechanical (a new required field at existing construction sites), not scope creep;
  baseline bumped, no split attempted — out of scope for this phase.
- Phase 6 (`smelt explain`) still needs the `smelt explain` rendering the outcome's §Known
  Divergences sentence in `sources.md` flags as unbuilt — untouched by this phase, as planned.
- No UI call site consumes the new reporter callbacks yet (`smelt-ui`'s reporter inherits the
  no-op defaults) — wiring the UI event stream was explicitly out of scope per the plan.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN
- `cargo test -p smelt-runtime --test external_step_reporting --test external_step_invocation --test execute_parity` — 4+7+6 passed
- `cargo test -p smelt-state` — 310+2+5 passed
- `cargo test -p smelt-cli --test run_report --test cli_docs_coverage` — 3+3 passed
- `cargo test -p smelt-core --test hardening_budget` — 5 passed (ratchet updated for `smelt-cli println` 175→176, sign-off note added)
- `bash .claude/scripts/large-file-check.sh` — OK (baseline updated for 3 mechanically-grown files)
