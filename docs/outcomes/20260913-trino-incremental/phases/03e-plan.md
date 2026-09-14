# Phase 3e plan — live-Trino test isolation

## Objective

Make every live-tier Trino test fail only for real reasons: give each one a schema name that is
guaranteed unique (not time-derived), and close the `TRINO_ENV_GUARD` lock-scope hole that lets
`trino_state_residency.rs` stage a `smelt.yml` with no `trino:` target. Serves criterion 11 (gates
green) and de-risks criteria 2 and 7 — a conformance or family gate that fails on a schema race is
indistinguishable from one that fails on a correctness defect, and 3f–3h/4–8 all add live legs on
top of this harness.

## Spec delta

None. Test-harness only; no user-visible behaviour changes, so the spec-first rule does not bite.

## Root cause (measured by 3c, re-read here)

- `crates/smelt-cli/tests/common/mod.rs::trino_schema` returns `smelt_seed_{label}_{pid}` and
  `crates/smelt-backend-trino/tests/common/mod.rs::unique_schema` returns
  `{base}_{suffix}_{pid}_{subsec_nanos}`. Neither is unique: `capability_probes.rs` passes the same
  `"cap"` suffix from 14 concurrent tests, so two tests can draw the same name, and the loser's
  `DROP SCHEMA … CASCADE` deletes the winner's tables ("Namespace already exists" / "does not
  exist", both observed). A process-wide mutex cannot fix this — the racing tests are separate
  binaries, i.e. separate processes — so guaranteed-unique naming is the mechanism.
- `trino_residency_legs_skip_not_pass_when_url_unset` removes `SMELT_TRINO_URL` under
  `TRINO_ENV_GUARD`, but `stage_residency_project` → `trino_target_block` reads it without the
  guard, yielding an empty target block and `targets: invalid type: unit value`.

## Tests

1. `trino_ci_wiring.rs::trino_schema_names_are_unique_within_a_process` — 1000 calls to
   `common::trino_schema("x")` yield 1000 distinct names. Red today (all identical).
2. New offline binary `crates/smelt-backend-trino/tests/schema_isolation.rs::
   unique_schema_names_never_repeat_for_one_suffix` — 1000 calls to `common::unique_schema("cap")`
   are all distinct. Keep the file free of the live-gate markers (`live_env_or_skip(`, `trino_env(`,
   `targets_to_run_with_trino(`, a raw `SMELT_TRINO_URL` read) so 3d's derived census correctly
   classifies it offline and does not demand a CI job entry.
3. `schema_isolation.rs::a_generated_schema_name_is_a_legal_lowercase_identifier` — length within
   Trino's identifier limit, `[a-z0-9_]` only, first char alphabetic; both helpers' output checked
   through one shared assertion so they cannot drift apart.
4. `trino_state_residency.rs::residency_project_yaml_always_carries_a_trino_target` (live-gated on
   `has_trino_env()`, filesystem-only — no coordinator call): a background thread runs the
   URL-unset leg's remove/restore cycle in a loop while the main thread stages N projects; every
   staged `smelt.yml` contains `type: trino`. Red today.
5. `trino_ci_wiring.rs::every_live_trino_test_schema_name_comes_from_the_shared_helper` —
   structural, over the *derived* live-gated census 3d landed: no live-gated Trino test file builds
   a schema name from an inline `format!`/string literal; each obtains it from `trino_schema(` or
   `unique_schema(`. This is the anti-regression gate.

## Tasks

1. Rewrite `trino_schema` and `unique_schema` over one naming rule: `{base}_{label}_{pid}_{n}_{r}`
   where `n` is a process-local `AtomicU64` counter and `r` a 64-bit random/nanos entropy suffix,
   truncated to a legal identifier; document that time alone is not a uniqueness source.
2. Land tests 1–3 red, then green.
3. Widen `TRINO_ENV_GUARD` in `trino_state_residency.rs` so `stage_residency_project` resolves the
   target block under the guard (preferred: resolve `trino_target_block` once while holding it and
   pass the block in, keeping the guard off the filesystem work).
4. Audit `trino_lock_versioning.rs` and `trino_smoke.rs` for the same unguarded
   `trino_env`/`trino_target_block` reads beside their own URL-unset legs; fix any found the same
   way. Record in the summary if there are none.
5. Land tests 4–5 red, then green.
6. Fix the stale header claim in `trino_state_residency.rs` that Trino has no `MaintenanceDialect`
   (3c's carried-over doc correction), and note the new naming rule in both `common/mod.rs` docs.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test trino_ci_wiring`
- `cargo test -p smelt-backend-trino --test schema_isolation`
- Live tier (`bash scripts/trino-up.sh`; `source scripts/trino-env.sh`), at **default parallelism —
  no `--test-threads=1`**, three consecutive green runs of each:
  `cargo test -p smelt-backend-trino` and
  `cargo test -p smelt-cli --test trino_ddl_live --test trino_state_residency --test
  trino_lock_versioning --test trino_incremental_families --test trino_smoke`, plus
  `cargo test -p smelt-cli --test seed_parity --test materialization_parity --features duckdb`.
  Three green runs at default parallelism is this phase's acceptance evidence; record the commands
  and counts in the summary. If the coordinator is unreachable, emit `<<PHASE_BLOCKED>>` — never
  skip green.
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity` (unchanged, proves no
  collateral).

## Commit message

`test(trino): give every live-tier test a guaranteed-unique schema and close the residency env-guard hole`
