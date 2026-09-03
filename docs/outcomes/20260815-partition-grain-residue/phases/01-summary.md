# Phase 1 summary — audit

**Verdict table:** see `docs/outcomes/20260815-partition-grain-residue/audit.md` (12 rows, one
per partition-grain Known Divergences bullet). Short form:

| Residue | Verdict | Phase |
|---|---|---|
| Lookback `NotDerivable` gate reads outer SQL only | OPEN | 2 |
| Batch-safety `OVER`/`RANGE` check on unexpanded SQL | PARTIALLY LANDED (explain path fixed; bare analyzer call still open) | 2 |
| CTE-only `event_time_column` detection | OPEN | 3 |
| Per-`ModelDef` overrides | OPEN | 4 |
| Monotone-integer `partition_column` end-to-end run | OPEN | 5 |
| Per-source clamp observability (`explain --json` run-relative bounds) | OPEN | 6 |
| `partition_column` rename refusal | OPEN | 7 |
| data_latency, non-det membership, ForbidsMetrics, sub-g_part suggestion, NOW()/CURRENT pinning | not probed — no `docs/plans/*` tracker predates `docs/outcomes/`; out of this outcome's scope | none |

No phase row closed early — every in-scope residue is confirmed still open (or, for #2, only
partially landed) by a pinned, currently-passing probe.

**Probe files:**
- `crates/smelt-logical/tests/partition_residue_probes.rs` — `probe_lookback_gate_sees_define_body`,
  `probe_batch_safety_sees_over_in_define_body`, `probe_cte_only_event_time_column`,
  `probe_modeldef_per_model_override`.
- `crates/smelt-cli/tests/partition_residue_probes.rs` — `probe_integer_partition_column_run`,
  `probe_explain_json_run_relative_source_bounds`, `probe_partition_column_rename_refusal`.

All 7 probes pass today (i.e. pin the divergent/incomplete behaviour). Each probe's doc comment
states the inversion condition phases 2–7 must satisfy to close it, and each asserts loudly (panic
message naming the spec section to update) if the residue turns out already landed.

## Decisions

- No phase row added for the five out-of-scope bullets (data_latency, non-deterministic
  membership, `PartitionGrainForbidsMetrics`, sub-`g_part` suggestion, `NOW()`/`CURRENT_*`
  pinning) — none cites a `docs/plans/*` tracker predating `docs/outcomes/`; folding them in would
  widen the outcome beyond its own framing. Recorded in `outcome.md`'s Decision log.
- `analyze_batch_safety`'s residue (#2) is only *partially* open: `smelt explain`'s own call site
  already pre-expands via `FnBodyMap` (from the prior `20260530-thread-fn-registry-classification`
  plan), so the probe pins the bare function call, not the explain surface itself. Phase 2 should
  scope its fix to the callers that still don't pre-expand (the `NotDerivable` refusal gate,
  `smelt backbuild`) rather than re-doing already-landed work.

## For the next planner (phase 2)

- **Most surprising finding:** the spec's own claim for bullet #3 ("specified ahead of a tracking
  plan") is stale — it's tracked by `20260704-model-updates-l4-batched.md` Phase BL8 (`pending`).
  Phase 8's close-out must fix this line, not just remove the divergence.
- Two of phase 5/6's residues (#3, #11) are tracked by the *same* plan
  (`20260704-model-updates-l4-batched.md`) under different phase names (BL8, BL6) — BL6's `done`
  status only ever closed the narrow trace/bound-admission slice, not the end-to-end run; don't
  assume "done" in a cited plan means the spec bullet is closed without checking the phase's own
  stated scope (this cost real time in this audit).
- Phase 2's probe (`probe_lookback_gate_sees_define_body`) targets `derive_model_source_bounds`
  called from `crates/smelt-runtime/src/safety.rs:115` (`check_bound_derivation`) with raw
  `model.content` — this is the actual fix site, not `smelt-planner` (already just a re-export).
- `analyze_one_select`'s "does not descend into subqueries" doc comment (`smelt-logical/src/
  analysis/temporal.rs`) means even a *fully expanded* SQL string can hide an `OVER` inside a
  derived table produced by function expansion — the `RANGE BETWEEN INTERVAL` text-scan mitigation
  helps only for that specific bounded-frame shape, not `LAG`/`LEAD` or default-frame windows.
  Worth flagging to phase 2 as a residual gap beyond what expansion alone fixes.

## Gates

- `cargo test -p smelt-logical --test partition_residue_probes` — 4 passed.
- `cargo test -p smelt-cli --test partition_residue_probes` — 3 passed.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, workspace tests, example_diagnostics).
