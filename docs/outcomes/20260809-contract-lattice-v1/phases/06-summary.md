# Phase 6 summary — conformance oracle parameterised per lattice point

**Shipped:**
- `smelt_logical::contract` gained `ContractPoint` (`Default`/`FrozenHorizon { h }`/
  `Deferral { d }`), `OracleObligation` (`Exact`/`ExactOverRestrictedS`/`Bracketed`), and the pure
  dispatch/transform functions `oracle_obligation`, `restrict_run_window` (delegates to
  `frozen_horizon::clamp_write_range`), and `settled_cutoff` (delegates to a new
  `deferral::settled_cutoff`, `input_frontier - d`).
- `smelt-maintenance-testkit`: `STracker::s_at_for_point`/`s_at_settled`/
  `materialize_s_for_point`/`materialize_s_settled` — `s_at`/`materialize_s` are now thin wrappers
  delegating with `ContractPoint::Default`. `ModelRecipe.contract: Option<ContractDecl>`
  (`FrozenHorizon`/`Deferral` day-valued variants, default `None`); `render.rs` emits the
  `contract:` frontmatter block when present.
- `maintenance_conformance/gate.rs`: `assert_equivalence_at_point`/
  `assert_equivalence_at_point_with_frontier`, dispatching on `oracle_obligation` — `Exact`/
  `ExactOverRestrictedS` reuse the existing both-direction `multiset_equal_via_backend`;
  `Bracketed` asserts `maintained ⊆ full_refresh(S)` and `full_refresh(S_settled) ⊆ maintained` as
  two one-directional `EXCEPT ALL` checks. `assert_equivalence` now delegates with
  `ContractPoint::Default` — its signature and behaviour are unchanged for every existing caller.
- New `maintenance_conformance/contract_points.rs` (registered in `main.rs`): 3 tests — a
  default-point harness self-check, a frozen-horizon fixture (late row into an already-frozen
  partition; relaxed oracle holds, strict oracle asserted to FAIL), and a two-model deferral
  fixture (mirrors `contract_deferral_skip_e2e.rs`'s shape) proving a licensed skip's bracket holds
  across three run steps.
- Spec: `docs/specs/incremental_models.md` §Known Divergences — dropped the gate-parameterisation
  half of the contract-lattice bullet; kept the `explain`-rendering gap and the per-cell-deferral
  note.
- `crates/smelt-logical/tests/contract_lattice_spec.rs` gained
  `conformance_gate_consumes_the_oracle_transform` (structural — scans
  `smelt-maintenance-testkit/src` + `maintenance_conformance/` for calls to the three shared
  transforms and forbids local per-point arithmetic outside comments).

**Decisions:**
- All three plan-stated design decisions (pure restriction not a per-point comparator; deferral
  oracle as a bracket; gate keeps its one `EXCEPT ALL` comparator) implemented as decided.
- **`s_at_for_point`'s tracker-recorded window need not belong to the model under test.** The
  deferral fixture records the shared source's `upstream_advancer`-driven window into the SAME
  tracker used for `deferred_model`'s own oracle — `S` represents "what became visible in the
  source", not "what this specific model wrote", so this is legitimate and is how the bracket's
  `full_refresh(S)` leg can differ from a skipped model's own (empty) run history.
- **Settled-cutoff filtering is strict-less-than (`event_time < cutoff`), not `<=`.** Frontier
  values are exclusive-end day counts throughout this codebase (matches `IntervalStore`'s own
  `latest_date` convention and `frozen_horizon`'s "strictly before" framing); at the licensed-skip
  boundary (`lag == d`), `settled_cutoff == maintained_frontier` exactly, so `<` is required for
  `S_settled` to match what a correctly-behaving skip already reflects — `<=` would have
  overclaimed one extra day and made the bracket's leg-2 spuriously fail on the very case it exists
  to license.

**For the next planner:**
- Phase 7 (`explain` contract rendering, docs-site) is unaffected and remains next; it is the last
  row in the table.
- Nothing was deferred out of this phase's own scope.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings, full workspace
  `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-logical --test contract_lattice_spec` — 12 passed.
- `cargo test -p smelt-maintenance-testkit` — 30 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 70 passed (67 pre-existing + 3 new).
- `cargo test -p smelt-runtime --test contract_deferral_probe --test contract_frozen_horizon_clamp --test contract_late_arrival_probe --test contract_deferral_schedule`
  — unregressed (4 + 2 + 3 + 5 passed).
