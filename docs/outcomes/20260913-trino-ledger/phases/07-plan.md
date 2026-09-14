# Phase 7 plan — The staged relation group without temp tables

## Objective

T1 measured `supports_staged_relation_group` **true** on Trino (`crates/smelt-backend-trino/
tests/capability_probes.rs:231` asserts the CREATE/INSERT/SELECT/DROP group succeeds), so this is
the realise branch of criterion 8, not the by-name-refusal branch. Trino has no session temp
namespace and (phase 1, measured) no transactional write at all, yet every staged emitter
hardcodes `CREATE TEMP TABLE` and declares `transactional: true` — a claim the Trino backend's
default `execute_statement_group` (`crates/smelt-backend/src/lib.rs:550`, sequential, no
transaction, no warning) silently does not honour. This phase makes the staged relation's
**residence, name and lifecycle** one piece of derived capability data, owned once, so the group
is executable on a non-temp, non-atomic backend and an interruption between the stage and the
apply is recoverable. Advances criterion 8; the fail-loud leg also serves criterion 11.

**Scope boundary.** The staged relation is a *bookkeeping object* — its name, where it lives, who
drops it. The statement spellings of the maintenance families that use it stay
`20260913-trino-incremental`'s. No `MaintenanceDialect::Trino` variant is added here (phase 4's
standing decision), and no test routes through `execute_project`: phase 6 established that any
incremental write on Trino hard-errors downstream at `UnsupportedMaintenanceDialect` today, so the
live leg tests the relation lifecycle directly through `TrinoBackend`, which needs no maintenance
dialect.

## Spec delta (spec-first, lands before the code)

1. `docs/specs/multi_backend.md` §"Column-scoped merge and conditional-write capabilities" —
   `supports_staged_relation_group` today conflates two facts. Split them and promote both from
   spec-only to struct fields: **`staged_relation_residence`** (`SessionTemporary` — DuckDB,
   dropped implicitly with the session; `TargetSchema` — Trino/Iceberg, a real explicitly-named,
   explicitly-dropped table in the target's own schema) and
   **`staged_relation_group_is_atomic`** (`true` DuckDB, `false` Trino). Update §Known
   Divergences: the flag is no longer spec-only.
2. `docs/specs/model_transforms.md` §"The staged-candidate conditional DELETE+INSERT" and its
   §Known Divergences entry — restate "temp relation" as "staged relation, whose residence is a
   backend capability", and state the three obligations a **non-atomic** backend carries, since
   the one-transaction guarantee is unavailable there: (a) every stage statement precedes any
   target mutation, so an interruption before the apply leaves the target byte-unchanged;
   (b) the relation's name is *derived*, deterministic per (purpose, target table) and prefixed so
   it can never collide with a user model; (c) the group reclaims its own relation with a leading
   `DROP ... IF EXISTS` and repopulates it before reading it, so an orphan from an interrupted run
   is never adopted as live data. Concurrent runs of the same model are excluded by the state
   lock, not by the name — phase 8's subject, named here as the dependency.
3. `docs/specs/state.md` §"The degradation contract" — one sentence: a statement group a backend
   cannot run atomically is emitted `transactional: false` with the recovery obligations above,
   never emitted `true` and silently executed one statement at a time.

## Tests (red-green)

- `smelt-logical` `staged_relation::name_is_derived_once_for_every_purpose` — the three live
  spellings (`__smelt_staged_*`, `__smelt_diff_patch_*`, `repair_staged_relation`) all come from
  the one deriver; a dotted qualified table flattens to a single non-colliding identifier.
- `smelt-logical` `session_temporary_residence_emits_create_temp_table` — DuckDB's existing
  byte-exact statement assertions in `emit/staged.rs` and `emit/recompute.rs` stay **byte
  identical**. This is the non-regression gate; it must not be relaxed.
- `smelt-logical` `target_schema_residence_emits_a_schema_qualified_real_table` — no `TEMP`, name
  qualified by the target schema.
- `smelt-logical` `non_atomic_residence_prepends_a_reclaim_drop_and_flags_the_group_non_atomic` —
  leading `DROP TABLE IF EXISTS`, trailing `DROP TABLE`, `transactional == false`; and the
  ordering assertion that no statement mutating the target precedes the last stage statement.
- `smelt-dialect` `every_capability_profile_declares_a_staged_relation_residence` — exhaustive
  over `duckdb/spark_delta/spark_parquet/bigquery/trino_iceberg`, the constructor list phase 5
  found `trino_iceberg()` missing from elsewhere.
- `smelt-runtime` `no_non_atomic_backend_is_handed_a_transactional_group` — the fail-loud leg: a
  group built for a `staged_relation_group_is_atomic == false` capability never carries
  `transactional: true` into `Backend::execute_statement_group`'s silent sequential default.
- `smelt-backend-trino` (live) `staged_relation_lifecycle_survives_interruption_between_stage_and_
  apply` — execute the emitted group's stage statements against the live coordinator, stop before
  the apply, assert the target table is unchanged and exactly one orphan staged relation exists;
  re-run the whole group from the top, assert it succeeds *despite* the orphan, the target is
  correct, and no staged relation remains afterwards.

## Tasks

1. Land the three spec edits above.
2. New single-owner module `crates/smelt-logical/src/maintenance/emit/staged_relation.rs`:
   `StagedRelationResidence`, `StagedRelation { name, residence, atomic }`, a pure
   `StagedRelation::derive(purpose, qualified_table, residence, atomic)` and its
   `create_prefix()` / `reclaim_statement()` / `drop_statement()`.
3. Add `staged_relation_residence` and `staged_relation_group_is_atomic` to `BackendCapabilities`
   and set them in all five constructors (`crates/smelt-dialect/src/dialect.rs`).
4. Re-key the four staged emitters (`emit/staged.rs` ×3, `emit/recompute.rs` ×2) from
   `staged_relation: &str` to `&StagedRelation`, deriving the CREATE spelling, the reclaim step
   and `StatementGroup::transactional` from it. No dialect branch: this is capability data.
5. Re-key the ad hoc name sites — `smelt-runtime/src/cumulative.rs:216`,
   `maintenance_driver/delta_restriction/mod.rs:75`, `maintenance_driver/repair/execute.rs:207`
   and `:214` — to the deriver, keeping each purpose's existing prefix so DuckDB names are
   unchanged.
6. Write the live Trino lifecycle test. Bring the tier up first (`bash scripts/trino-up.sh`,
   `source scripts/trino-env.sh`); if the coordinator is unreachable, emit `<<PHASE_BLOCKED>>`
   rather than letting the test skip green.
7. If `staged.rs`/`recompute.rs` cross `.claude/large-file-baseline.txt`, split rather than bump.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --lib maintenance::emit::`
- `cargo test -p smelt-dialect`
- `cargo test -p smelt-runtime --test statement_parity` (the single-owner gate — the emitters moved)
- `cargo test -p smelt-cli --test maintenance_conformance` (DuckDB end-state unchanged)
- `cargo test -p smelt-backend-trino --test staged_relation_lifecycle` (live tier up)
- `bash .claude/scripts/large-file-check.sh`

## Commit message

`feat(state): the staged relation group without temp tables — residence, derived name and reclaim as capability data`
