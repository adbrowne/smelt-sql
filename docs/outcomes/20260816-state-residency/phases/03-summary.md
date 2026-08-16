# Phase 3 summary — absent-state runtime behaviours

## Shipped

- Task 1: repaired the second pre-existing red-gate class. `crates/smelt-logical/tests/{output_delta_spec,typed_edge_spec}.rs`'s `section_body` lookups now search only after `## Semantics`, so they can no longer match the Overview primer's one-paragraph restatement of "### The graph layer" (`docs/specs/incremental_models.md:163`) instead of the normative body (`:1258`). `incremental_models.md` §"The graph layer" now names the `KeyedUpsert`/`General` output-delta verdicts explicitly (backticked, capitalized, cross-referenced to `model_properties.md` §"Output-delta shape") alongside its existing lowercase prose — the judgment call: the section's `general`/`keyed upsert` really do name the same `OutputDelta` verdict `model_properties.md` owns, so the fix is a spec edit naming that owner, not a test-side weakening.
- `smelt_state::ProbeRecordOutcome::BaselineEstablished` — new serde variant (`crates/smelt-state/src/lib.rs`), serializes as `"baseline_established"`.
- `RunReporter::probe_advisory(run_id, model, code, message)` — new default-no-op hook (`crates/smelt-runtime/src/reporter.rs`); `CliReporter` prints it as `smelt: warning: ...`; `smelt-ui`'s `BroadcastReporter` inherits the no-op default (it surfaces no other warnings today either).
- `source_probes.rs`'s `Establish` arm and `contract_probes.rs`'s `recorded.is_empty()` branch now record `BaselineEstablished` (was `Dispatched`) and call `reporter.probe_advisory` with `ProbeBaselineUnavailable`, naming the source and the probe's cell/partition-set description.
- `execute.rs`'s `EventSink`/`ReporterEvent` per-model buffering (`docs/plans/20260719-prod-w2-operability.md` Phase 5) gained a `ProbeAdvisory` variant + replay arm — without it, advisories from concurrent model execution units were silently swallowed (caught by a debug test before shipping).
- `smelt diff` / `crates/smelt-cli/src/commands/diff.rs`: `ModelDiffStatus::New` now carries `snapshot_absent_reason: Option<String>`, set when `state.mode: stateless` excludes schema snapshots. Text output prints a `⚠` say-so line; `--json` adds `"snapshot_absent_reason"`. A snapshot missing because its file was deleted under a snapshot-writing posture stays a plain `new`.
- Spec delta: `docs/specs/run_state.md` §"Run manifest" — probe-record `outcome` vocabulary gains `"baseline_established"`.
- New tests: `smelt-state`'s `probe_record_serde_round_trips_baseline_established`; `smelt-runtime`'s `first_observation_records_baseline_established_and_reports_advisory` / `second_run_verifies_and_reports_no_advisory` (`source_probes.rs`), `frozen_band_first_observation_reports_baseline_unavailable` / `deferral_never_reports_baseline_unavailable` (`contract_late_arrival_probe.rs`), `stateless_posture_reports_baseline_unavailable_every_run` (`state_posture.rs`); `smelt-cli`'s new `tests/e2e/absent_schema_snapshot.rs` (2 tests).

## Decisions

- The graph-layer verdict-naming call: spec-side fix naming `KeyedUpsert`/`General` explicitly, not a test weakening — see task 1 above and the reasoning already recorded in the phase plan.
- `probe_advisory` reports regardless of `state.mode` (unlike every other `RunReporter` callback, which is cosmetic) — the optionality rule requires degradation be said even under `stateless`, where no manifest is ever written to carry it durably.
- `.claude/hardening-baseline.txt`'s `smelt-cli println` count moved 161 → 163: the ratchet's substring match counts `eprintln!` as `println!` (it contains that substring), so the new `println!` (diff say-so line) and the new `eprintln!` (probe-advisory warning) both counted. Both are intentional user-facing output in `smelt-cli`, which the "no `println!` in libraries" gate already excludes — updated via the gate's own `--update` remediation path rather than treated as debt.

## For the next planner

- Task 1's spec edit was scoped to the minimum needed to pass the two tests (naming the verdicts in one paragraph). The rest of "The graph layer" section still uses lowercase `general`/`keyed upsert` prose throughout — consistent as informal shape-lattice language, not inconsistent with the fix, but worth knowing if a future spec pass wants uniform capitalization.
- `docs/specs/incremental_models.md` and `model_properties.md` both have an Overview-primer heading that shadows a real `##Semantics` section heading (`### Delta signatures` also appears twice, at line 92 and 549, though no standing gate currently depends on disambiguating that one). If a future spec-authored test needs `section_body("### Delta signatures")`, it will hit the same first-match bug — worth a shared, disambiguating test helper if this pattern recurs a third time.
- `crates/smelt-ui`'s `BroadcastReporter` still has no `probe_advisory` override — it inherits the no-op default. If the UI ever wants to surface run warnings, this is the hook to wire; out of scope here since no other warning surfaces there either.
- Phase 4 (engine-resident reconciliation ledger) is next per the outcome's phase table; nothing this phase touched depends on it.

## Gates

- `bash .claude/scripts/verify-phase.sh` (full) — ALL GREEN (after `hardening-budget.sh --update` for the println baseline).
- `cargo test -p smelt-logical --test output_delta_spec --test typed_edge_spec` — 8/8 pass.
- `cargo test -p smelt-runtime --test source_probes --test state_posture --test contract_late_arrival_probe --test probe_manifest` — 21/21 pass.
- `cargo test -p smelt-state` — pass (264 + new round-trip test).
- `cargo test -p smelt-cli --test e2e absent_schema_snapshot` — 2/2 pass.
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity` — 27/27 pass.
- `cargo test -p smelt-cli --test maintenance_conformance` — 70/70 pass.
- `git diff --stat -- docs/specs/` — only `incremental_models.md` (task 1) and `run_state.md` (spec delta), as expected.
