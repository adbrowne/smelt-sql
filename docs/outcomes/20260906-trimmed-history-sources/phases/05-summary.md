# Phase 05 summary — The bound moving is an event

## Shipped

- `MaintenancePlan::retention_reaches: Vec<RetentionReach>` (`crates/smelt-logical/src/maintenance/plan.rs`) — the bounded reach-versus-retention proof (`Within`/`Exceeds` alike), carried alongside `retention_downgrades`.
- `smelt-logical/src/maintenance/retention.rs`: `retention_reaches()` (collects the bounded verdicts), `retention_refusals_at_age(reaches, window_age)` (pure fold, ages `required_lookback` and refuses what now exceeds), `run_window_age(window_start, now)` (saturating day-granularity age). All three plus `RetentionReach` re-exported from `smelt_logical::maintenance`.
- `derive_maintenance_plan_impl` (`maintenance/derive/plan.rs`) now populates `plan.retention_reaches` from the same window-age-zero verdict fold that already produced `retention_downgrades`/refusals — no new derivation, no re-walk.
- New `crates/smelt-runtime/src/execute/retention_admission.rs`: `derive_model_retention_plan` (derives a model's plan through `maintenance_availability::derive_resolved` — the one legal seam — purely to read its retention fields), `check_retention_admission` (pure fold: age the plan's `retention_reaches` by the run's own window age, refuse the first exceeding one), `downgrade_warning_message` (renders a `RetentionDowngrade` as reporter-warning text).
- `execute_project`'s per-model path (`crates/smelt-runtime/src/execute/project/mod.rs`, right after `reporter.model_started`) now runs this check **before any statement is compiled or executed** for the model, and reports every `retention_downgrades` entry as a `RunReporter::maintenance_warning` call.
- New `RunReporter::maintenance_warning(run_id, model, message)` trait method (default no-op) plus `EventSink`/`ReporterEvent::MaintenanceWarning` wiring so the wavefront scheduler's buffered replay carries it.
- `crates/smelt-runtime/tests/retention_admission.rs` — 3 real-DuckDB `execute_project` tests (plan's tests 6-8): an aged backfill window refuses before any statement runs and leaves the target table absent; a forward-only run over the same model still succeeds; an unbounded-reach model's downgrade is reported exactly once via `maintenance_warning`.
- `crates/smelt-logical/src/maintenance/retention.rs`'s `rolling_tests` module — 5 unit tests (plan's tests 1-5): `retention_reaches` totality, age-crossing refusal, monotonicity (age never rescues an exceeding reach), the zero-age-saturation of `run_window_age`, and a plan-purity round-trip check.
- Spec: `docs/specs/sources.md` §Semantics 5 and `docs/specs/model_properties.md` §"Reach versus retained history" now state the run's required look-back is `derived reach + the age of the oldest region the run writes`, and that the refusal fires before any statement executes, leaving stored output untouched.

## Decisions

- The plan-derivation-time verdict fold (`window_age: Seconds::ZERO`, unchanged from phase 4) stays the LSP/analysis-time refusal source; `retention_reaches` is a *new*, separate field carrying the un-aged bounded proof for the run to fold its own age onto later — two consumers of one derivation, not two derivations.
- `derive_model_retention_plan` in `smelt-runtime` must go through `maintenance_availability::derive_resolved` (`StateAvailability::all()`, since retention derivation ignores availability entirely) rather than calling `smelt_db::queries::maintenance::derive_model_maintenance_plan` directly — `availability_seam`'s structural gate enforces exactly one call site for the raw derivation, discovered red the first time this landed.
- `agg`/`unbounded` test fixtures use a bounded/unbounded `RANGE BETWEEN ... PRECEDING` window frame to derive reach, not a `WHERE col >= CURRENT_DATE - INTERVAL '...'` predicate — `CURRENT_DATE` type-checks as `UndeclaredColumn` in the current dialect surface (confirmed against `examples/broken/models/retention_exceeded.sql`, which carries the same diagnostic unfiltered), which is orthogonal to retention but trips `execute_project`'s pre-execution diagnostics gate. The window-frame pattern is the same one already covered by phase 3's unit tests.

## For the next planner

- **Known gap, not exercised by this phase's tests**: `derive_model_retention_plan` passes `driving_source_granularity: None` and `explicitly_mutable: HashSet::new()` — correct for every `grain: partition` model (the only shape phase 5's tests cover), but a `grain: key` model with its own `timeseries:` block could have its plan derivation short-circuit into `locality_refused_plan` (empty cells, and critically empty `retention_reaches`) before reaching the retention fold at all, since the keyed-locality gate needs a real driving-source granularity to establish. That would silently skip the rolling re-evaluation for such a model. Phase 6 (whole-table recompute refusal) or a follow-up should confirm whether this matters in practice for the phase's own scope, or plumb the real granularity through.
- Phase 6's own scope (`--full-refresh`/first-build over a trimmed source reaching past every finite bound) is untouched by this phase — `derive_model_retention_plan`/`check_retention_admission` only fold a *finite* window age; a full-refresh run has no window at all today, so it is unaffected by this check and still needs phase 6's dedicated refusal.
- `RunReporter::maintenance_warning` is new surface with no CLI/UI presentation yet (only `EventSink` and the test's `CapturingReporter` implement it beyond the default no-op) — phase 8 (explain/docs) should decide whether `smelt-cli`'s terminal reporter should print it, or whether `smelt explain` is the only intended surface for retention downgrades (the existing LSP diagnostic already covers analysis-time visibility).

## Gates

- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy both feature sets, full workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-logical --test walk_coverage` — PASS (14 tests).
- `cargo test -p smelt-runtime --test execute_parity --test statement_parity --test availability_seam` — PASS (41 + parity/availability tests, including the structural single-seam gate).
- `cargo test -p smelt-db --test integration diagnostics_catalogue` — PASS.
- `cargo test -p smelt-cli --test example_diagnostics` — PASS (128 tests, 1 ignored).
- `bash .claude/scripts/large-file-check.sh` — PASS after `--update` (`execute/project/mod.rs` baseline moved 4796 → 4827 lines, the +31 this phase's insertion adds; reviewer sign-off: exactly the plan's own task 5 insertion, no unrelated growth).
