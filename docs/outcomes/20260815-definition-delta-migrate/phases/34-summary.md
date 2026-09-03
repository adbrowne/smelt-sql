# Phase 34 summary — persist the `retain_departed` probe outcome on the run manifest

**Shipped:**
- `smelt_state::ProbeRecord` gains `observed: Option<u64>` (`#[serde(default, skip_serializing_if
  = "Option::is_none")]`) — `crates/smelt-state/src/lib.rs`. All 11 existing `ProbeRecord`
  construction sites (`contract_probes.rs`, `model_probes.rs`, `source_probes.rs`) updated to
  `observed: None`.
- `execute_snapshot_reconcile` (`crates/smelt-runtime/src/cumulative.rs`) takes a `probe_sink:
  &mut Vec<ProbeRecord>` out-parameter; the `Retain` arm pushes a `ProbeRecord { fact:
  "contract.retain_departed", probe: "ContractDepartedKeyUnmarked", outcome: Dispatched, observed:
  Some(retained_count) }` — unconditionally, not cadence-gated (matches the spec's "dispatched on
  every reconcile that suppresses the delete" wording).
- `execute.rs`'s keyed dispatch declares `retain_departed_probes: Vec<ProbeRecord>` before the
  run-shape match, threads it into the snapshot-reconcile arm, and feeds it into the cumulative
  arm's `ModelRunRecord.probes` (previously always `Vec::new()`).
- The unmarked-tombstone refusal message is now prefixed `ContractDepartedKeyUnmarked: ...`.
- `probe_plan.rs::probe_plan_for_model` appends a `ProbePlanEntry` (fact
  `contract.retain_departed`, probe `ContractDepartedKeyUnmarked`) whenever the model declares
  `contract.retain_departed`, so `smelt explain` lists it.
- Spec deltas: `run_state.md` (`observed` field + JSON example), `incremental_models.md`
  (§Retention prose + diagnostics table row for `ContractDepartedKeyUnmarked`), `diagnostics.md`
  (table row + corrected "not yet dispatched" sentence). `cli.md` left unedited — its probe
  section describes facts generically, not per-fact.

**Decisions:**
- No `ProbePolicy`/cadence plumbing added to `execute_snapshot_reconcile` — the spec explicitly
  makes this probe cadence-independent (it stands in for the delete the default point would
  otherwise run), so there is no `Skipped` variant to produce here, unlike the other declared-fact
  probes.
- Kept the existing `retain_departed_probe_is_dispatched_pre_write` test and added the
  `ContractDepartedKeyUnmarked` name assertion into it directly, rather than duplicating the whole
  fixture under the plan's suggested test name — same coverage, less fixture duplication.

**For the next planner:**
- `frozen_horizon`/`deferral`'s probes are *not* currently listed in `probe_plan_for_model` either
  (checked while implementing this phase) — only the newly-added `retain_departed` entry and the
  pre-existing `key_recurrence`/`referential_integrity` rows exist. If `smelt explain` completeness
  for the full lattice is a goal, that's a gap beyond this phase's scope.
- `cli.md`'s `smelt explain` probe section was confirmed generic (no per-fact enumeration) — no
  edit was needed there; future probe additions should keep checking that assumption still holds.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-runtime --test departed_key_reconcile` — 5 passed.
- `cargo test -p smelt-runtime --test statement_parity` — 33 passed.
- `cargo test -p smelt-logical --test contract_lattice_spec` — 13 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 74 passed.
- `rg -n 'not yet dispatched by any live run' docs/specs/` — no matches.
