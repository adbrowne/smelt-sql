# Phase 5 plan — `supports_fingerprint_sidecar` becomes the sole owner of the sidecar gate

## Objective

Advances success criterion 5. The `incremental_models.md` §Known Divergences
"Conditional-maintenance gaps" bullet names `BackendCapabilities::supports_fingerprint_sidecar`
as the thing that decides whether a target gets the sidecar, and `multi_backend.md` says that
flag is "queried by admission exactly like every other capability flag — never re-derived by a
consumer". Only one consumer actually reads it today
(`resolve_live_external_delta_restriction_facts`); the sidecar's own six other gates re-derive it
as `dialect != SqlDialect::DuckDB`. This phase makes every gate read the flag, and states in the
spec what a flagless target actually gets on each of the two legs.

## Spec delta

`docs/specs/multi_backend.md` §"The fingerprint sidecar capability" — the bullet currently states
only the widened-scan consequence, which is a half-truth: that is the *external delta
restriction* leg's behaviour, while the repair-family / model-edge group-grain leg refuses with
`UnsupportedOnBackend` because a clamped current-source scan over a `mutable_snapshot` is
unsound, not merely wider. Rewrite the bullet to name both consequences explicitly, and to state
that the flag — not the target's dialect — is what every consumer reads. Add one clause noting
the sidecar's DDL owner (`smelt-state`'s `ddl_duckdb`) is DuckDB-shaped, so a second backend
setting the flag needs its own DDL first; `capability_conformance` pins the current matrix.

(`incremental_models.md`'s own Known Divergences bullet is phase 6's cleanup, not this phase's.)

## Tests

Red-green; each fails before the change.

1. `repair_lowering.rs::snapshot_discovery_refuses_without_the_sidecar_capability` — replaces
   `snapshot_discovery_fails_loud_on_a_non_duckdb_backend`; passes
   `BackendCapabilities::spark_delta().supports_fingerprint_sidecar` (false) rather than a bare
   literal, still expects `BackendError::UnsupportedFeature`.
2. `repair_lowering.rs::snapshot_discovery_admits_when_the_capability_is_declared` — a
   non-DuckDB dialect *with* `supports_fingerprint_sidecar: true` resolves
   `RepairDiscovery::SidecarDiff`. Proves the gate is the flag, not the dialect. Red today.
3. `key_addressed_model_edge_lowering.rs::key_addressed_edge_refuses_without_the_sidecar_capability`
   — same pair shape for `resolve_live_key_addressed_model_edge_cell`'s refusal leg.
4. `key_addressed_model_edge_lowering.rs::key_addressed_edge_admits_when_the_capability_is_declared`
   — non-DuckDB dialect + flag `true` resolves the cell. Red today.
5. `fingerprint_sidecar.rs::sidecar_entry_points_refuse_without_the_capability` — a stub
   `SidecarLessBackend` (dialect `DuckDB`, `capabilities()` = `duckdb()` with
   `supports_fingerprint_sidecar: false`) makes all four async entry points
   (`diff_fingerprint_sidecar_changed_keys`, `refresh_fingerprint_sidecar`,
   `diff_repair_group_sidecar_changed_keys`, `refresh_repair_group_sidecar`) return
   `UnsupportedFeature`. The sharpest red test: today all four proceed, because the DuckDB
   dialect alone satisfies their gate.
6. `capability_conformance.rs` — unchanged; re-run to confirm the declared flag matrix is
   untouched (DuckDB alone `true`).

## Tasks

1. Write the `multi_backend.md` spec delta first.
2. `maintenance_driver.rs`: add a `supports_fingerprint_sidecar: bool` parameter to
   `resolve_live_per_group_recompute_cell` (site ~2430) and
   `resolve_live_key_addressed_model_edge_cell` (site ~2878); gate on it instead of
   `dialect != SqlDialect::DuckDB`. Keep `dialect` — it still supplies `.name()` for the
   refusal message.
3. `maintenance_driver.rs`: in `diff_fingerprint_sidecar_changed_keys` (~3772),
   `refresh_fingerprint_sidecar` (~3879), `diff_repair_group_sidecar_changed_keys` (~4025) and
   `refresh_repair_group_sidecar` (~4129), swap the dialect comparison for
   `backend.capabilities().supports_fingerprint_sidecar`. No signature change — each already
   holds `&dyn Backend`.
4. Update the six doc comments that assert "DuckDB-only" to say "a target declaring
   `supports_fingerprint_sidecar`", and note at the `ddl_duckdb::generate_fingerprint_sidecar_
   table_ddl` call sites that the DDL owner is still DuckDB-shaped.
5. `execute.rs`: pass `backend.capabilities().supports_fingerprint_sidecar` at the two resolver
   call sites (~2144 and the key-addressed edge dispatch), alongside the existing
   `backend.dialect()`.
6. Update the existing tests in `repair_lowering.rs` / `key_addressed_model_edge_lowering.rs` /
   `observed_delta.rs` / `statement_parity.rs` for the new arity; add tests 1–5.
7. Leave the four `read_observed_delta` / ledger / merge dialect gates (sites 526, 1956, 3486,
   3609) alone — they are not sidecar gates, and re-homing them is a different capability
   question.

## Verification

- `cargo test -p smelt-runtime --test repair_lowering --test key_addressed_model_edge_lowering
  --test fingerprint_sidecar --test statement_parity --test observed_delta`
- `cargo test -p smelt-dialect --test capability_conformance`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `bash .claude/scripts/verify-phase.sh`

## Commit message

`feat(maintenance): gate every fingerprint-sidecar path on the capability flag, not the dialect`
