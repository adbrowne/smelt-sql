# Phase 8 plan — the generative gate's Trino arm, and the append-only partition pool live

**Outcome:** `docs/outcomes/20260913-trino-incremental/outcome.md`
**Advances:** criterion 7 (equivalence proved generatively on Trino, after every run step, under
actual availability, gated in `compat.yml`), criterion 8 (a downgraded cell is compared to the
oracle, never exempted), criterion 11 (gates green).

## Objective

Widen the generative conformance harness's single target seam to Trino — a `ConformanceTarget`
arm, not a duplicated harness — and prove it with the append-only partition pool's six
`families::gate` legs running live against the Docker tier, each asserting oracle equality after
every run step. Land the `maintenance-conformance-trino` CI job in the same commit so the gate
exists the moment the first family passes. Phase 8b widens the pool; this phase lands the seam
proved.

## Spec delta (first)

`docs/specs/multi_backend.md` §"Parity contract" → "Generative equivalence coverage": the
`ConformanceTarget` enumeration gains Trino (carrying the per-case schema a case isolates in,
derived rather than threaded, as BigQuery's dataset is), and the paragraph names the Trino leg's
gated-tier command (`cargo test -p smelt-cli --test maintenance_conformance_trino --
--test-threads=1`) alongside Spark's. Record there, in behaviour terms, that Trino's leg supplies
its own oracle relation because the engine has no session-scoped temporary view — the same hook
BigQuery already uses. Any family not yet in Trino's pool goes to §Known Divergences with a
pointer to phase 8b, per the existing "rollout … tracked incrementally" sentence.

## Tests (red-green)

Offline (run everywhere, no tier):
1. `maintenance_conformance_trino::backend::tests::the_full_refresh_twin_lands_in_different_storage_than_the_incremental_project`
   — a case's incremental project and its oracle twin resolve different `(target, schema)`.
2. `maintenance_conformance_trino::backend::tests::every_case_gets_its_own_schema`
   — `schema(case)` is distinct across cases and legal per `trino_ci_wiring`'s identifier rules
   (3e's isolation ruling: no shared namespace between live tests).
3. `smelt-maintenance-testkit` unit: `render::tests::trino_target_renders_a_trino_block_with_the_case_schema`
   — the Trino arm of `render_smelt_yml_for` emits a `trino:` target block naming the case schema;
   network-free.
4. `trino_ci_wiring::compat_workflow_has_a_maintenance_conformance_trino_job` — the new job exists,
   is gated exactly like `trino-integration`/`maintenance-conformance-spark`, brings the tier up,
   tears it down with `if: always()`, and fails loudly if a leg skips with the tier up.
5. `trino_ci_wiring::the_trino_job_runs_every_live_gated_trino_test_binary` (existing, widened) —
   the census accepts a live-gated binary that runs in EITHER Trino job, and still fails for a
   binary run by neither.
6. `trino_incremental_spec_freshness` — the spec's generative-coverage paragraph names Trino.

Live tier (must fail loudly, never skip green, when the tier is up):
7. `harness_self_check_trino::oracle_flags_a_seeded_divergence_on_trino` — the seeded corruption is
   caught, proving the oracle can fail on this backend before any green leg is believed.
8. `gate_trino::append_only_partition_pool_upholds_equivalence_on_trino` — deterministic case count
   (`SMELT_CONFORMANCE_TRINO_CASES`, default small, Spark's precedent).
9. `gate_trino::admission_rate_stays_above_floor_on_trino`.
10. `gate_trino::redelivery_of_processed_window_is_idempotent_on_trino`.
11. `gate_trino::full_refresh_interleave_resets_state_correctly_on_trino`.
12. `gate_trino::boundary_rows_within_reach_are_reflected_on_trino`.
13. `gate_trino::column_add_between_runs_recovers_equivalence_on_trino`.

## Tasks

1. Spec delta above, first.
2. `smelt-maintenance-testkit`: add the unconditional `smelt-backend-trino` dependency (it is
   already a non-optional dep of `smelt-backends`/`smelt-cli` — no feature flag, env gate only),
   and a Trino env reader mirroring `smelt-cli/tests/common/mod.rs::trino_env`'s convention
   (`SMELT_TRINO_URL`/`SMELT_TRINO_USER`/`SMELT_TRINO_CATALOG`); do not reach into the CLI's test
   module.
3. `recipe.rs`: `ConformanceTarget::Trino { schema }` plus a `trino_conformance_schema(family,
   case)` deriver in `bq_conformance_dataset`'s shape (pid + family + case, legal Trino identifier).
4. `render.rs`: Trino arms in `render_smelt_yml_for` (a `trino:` target body) and all three staging
   entry points (`stage_for_target`, the keyed one, the composed one) — each `CREATE SCHEMA IF NOT
   EXISTS` then seeds through the real `TrinoBackend`, matching the Spark arm's write-then-seed
   shape exactly.
5. `link_c_harness.rs`: Trino arms in the run and backend-open `match`es, plus
   `open_trino_conformance_backend(schema)`.
6. New `crates/smelt-cli/tests/maintenance_conformance_trino/` (`main.rs`, `backend.rs`,
   `gate_trino.rs`, `harness_self_check_trino.rs`) modelled on `maintenance_conformance_spark/`:
   `TrinoConformanceBackend` implements `target`/`schema`/`twin_*`/`engine_name`/`skip_reason`/
   `corrupt_sql`/`open_backend`/`dialect` (`BackendType::Trino` — `row_set_body` already has that
   arm) with no `storage_clause` (Iceberg needs none). Wrappers stay thin: env check, tokio runtime,
   call `families::gate::run_*`.
7. Measure, don't assume, two hooks against the live tier and record each verdict in the summary:
   whether Trino accepts `EXCEPT ALL` (keep the default `multiset_diff_sql` if it does, else
   override with the ranking emulation `oracle::bigquery_multiset_diff_sql` already provides), and
   the `oracle_relation` override (Trino has no session temp view — expect BigQuery's inline
   derived-table form; quote the measured error if the default is tried and refused).
8. `.github/workflows/compat.yml`: `maintenance-conformance-trino` job — Trino trigger discipline
   (`schedule` / `run-docker-tests` label / `needs.changes.outputs.trino`), `trino-up.sh` +
   `trino-env.sh`, run with `-- --test-threads=1`, the same `grep -qi skipping` loud-failure check
   the other Trino steps use, `trino-down.sh` with `if: always()`. Add the new test path to the
   `changes` filter's `trino` globs if the existing `crates/smelt-cli/tests/trino_*` glob does not
   already cover it (it does not — the binary is named `maintenance_conformance_trino`).
9. Widen `trino_ci_wiring.rs`'s job census to two Trino jobs, per test 5.
10. Run the live legs. If any family cannot pass for a reason owned by the product (not the test),
    fix it here; if it cannot pass for a reason this outcome cannot decide, stop and emit
    `<<PHASE_BLOCKED>>` with the measured error — a conformance gate that skips looks exactly like
    one that passes.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test trino_ci_wiring --test trino_incremental_spec_freshness --quiet`
- `cargo test -p smelt-maintenance-testkit --quiet`
- `cargo test -p smelt-cli --test maintenance_conformance --quiet` (the DuckDB leg is unchanged by
  the new enum variant)
- `cargo build -p smelt-cli --features smelt-cli/spark --test maintenance_conformance_spark`
  (the Spark leg still compiles against the widened seam)
- Live: `bash scripts/trino-up.sh && source scripts/trino-env.sh && cargo test -p smelt-cli --test
  maintenance_conformance_trino -- --test-threads=1`, then `bash scripts/trino-down.sh`. Any
  `skipping` line with the tier up is a failure, not a pass.

## Commit message

`feat(trino): land the generative conformance harness's Trino arm and the append-only pool`
