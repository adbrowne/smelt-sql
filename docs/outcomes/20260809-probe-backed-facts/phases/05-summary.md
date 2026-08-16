# Phase 5 summary — live dispatch of the three model-scoped probes

**Shipped:**
- `crates/smelt-runtime/src/model_probes.rs` (new, exported from `lib.rs`): `declared_model_probes(model_name, cell, metadata, timeseries, scope_select, dialect) -> Vec<DeclaredProbe>` — pure builder, one probe per declared `timeseries.assert_monotonic` / `functional_dependencies:` (one per entry) / `bounded_domain:`; and `dispatch_declared_model_probes(backend, policy, probes) -> Result<Vec<ProbeRecord>, BackendError>` — dispatches each through the shared `probes::dispatch_probe`, fails loud on the first `Violated` with a message opening with the registry diagnostic code and closing with `probe_violation_suffix`.
- Two live call sites in `crates/smelt-runtime/src/execute.rs`: the full-refresh `None => // Full refresh` arm (after `reporter.model_compiled`, before the materialization write) and the incremental batch loop (after `reporter.model_compiled`, before the DELETE+INSERT/merge dispatch). Both build the policy via the existing `probe_policy_for_model` and pass `&compiled.sql` as `scope_select`.
- `docs/specs/model_properties.md` §"Probe obligation": the three model-scoped rows' Status flips `built (unwired)` → `built`; §Known Divergences rewritten to name only the append-only posture probe (phase 6) as unwired.
- New tests: `crates/smelt-runtime/tests/model_probes.rs` (8 real-DuckDB tests on the pure builder/dispatcher) and `crates/smelt-cli/tests/e2e/declared_fact_probe_firing.rs` (3 tests through the real compiled binary — full-refresh FD violation, incremental-batch bounded-domain violation with pre-run-contents-unchanged, and `probes: {cadence: off}` letting the violating run write).

**Decisions:**
- Monotonicity's `partition_key` (the emitter needs a "per partition" grouping) is the model's declared `unique_key` when present, else falls back to the timeseries `partition_column` — always available on a model that can declare `assert_monotonic` at all, so the probe never has nothing to group by.
- `dispatch_declared_model_probes` returns `Vec<smelt_state::ProbeRecord>` (the phase-4 type) for held/skipped probes, not yet wired into `ModelRunRecord.probes` — matches the outcome's phase-8 plan to wire that once the fourth (phase 6) dispatch site exists too.
- Both call sites reuse `plan.model_file.metadata.as_deref()` and either `Some(&inc_plan.timeseries)` (batch site, always populated in that branch) or `metadata.timeseries.as_ref()` (full-refresh site) — no new metadata plumbing needed.

**For the next planner:**
- Phase 6 (append-only posture probe) is the only remaining `built (unwired)` row; the divergence note now says so explicitly.
- `ModelRunRecord.probes` population (phase 8, per the 2026-08-10 reshape note) still has no writer — this phase adds a fourth call site's worth of dispatch outcomes (`Vec<ProbeRecord>`) that phase 8's wiring should also consume.
- The e2e fixtures are self-contained inline-string workspaces (not new `examples/` dirs) — cheap to extend for phase 7's conformance recipes if a similar pattern is wanted, though phase 7 may prefer the `maintenance_conformance` harness's own recipe pool instead.
- Nothing found out of scope during this phase.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN
- `cargo test -p smelt-runtime --test model_probes --test probe_dispatch --test statement_parity --test execute_parity` — 39 passed
- `cargo test -p smelt-logical --test probe_obligation` — 6 passed (updated `built_and_unwired_rows_name_a_real_emitter` to expect `built` for the three model-scoped rows)
- `cargo test -p smelt-cli --test e2e --test example_diagnostics --test maintenance_conformance` — 171 + 119 + 59 passed
- `cargo test -p smelt-lsp --test example_workspaces` — 34 passed
