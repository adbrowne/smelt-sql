# Phase 8 plan — Locking and versioning: realised, not silently absent

## Objective

Advance success criterion 9. `.smelt/lock` and the `meta.json` version gate are
filesystem-resident, so on Trino they are **realised**, not refused — but a criterion
that says "never a lock that never locks" is only met once that is *demonstrated on a
Trino target* rather than inherited from DuckDB's tests. This phase proves, against the
live tier, that a second `smelt run --target trino` is refused by name while the lock is
held and writes neither `.smelt/` nor Iceberg; that a future `state_version` refuses the
same way; and it pins the one place the lock legitimately does not lock (`state.mode:
stateless`) as stated behaviour instead of silent behaviour.

## Spec delta (made first, by the implement step)

`docs/specs/run_state.md` §"Locking" — append one paragraph:

- The run lock and the layout-version check are **backend-independent**: they live under
  `.smelt/` on the invoking machine, depend on no backend capability, and are therefore
  realised identically on every target, including targets that claim no engine-resident
  correctness structure (`state.md` §"The residency rule"). No target refuses them, and
  no target's absence downgrades them — they are not correctness structures.
- Both fire in `execute_project` **before any state artifact is written**, so a refused
  second run leaves `.smelt/` and the warehouse untouched.
- The lock protects `.smelt/`, not the warehouse. Under `state.mode: stateless` nothing
  is written under `.smelt/`, so there is nothing to serialize: `lock()` is an explicit,
  specified no-op and two concurrent stateless runs both proceed. This is the absence of
  *state*, not the absence of a lock — smelt does not serialize concurrent writers to a
  warehouse table at any posture.

`docs/specs/state.md` §"The degradation contract" — one sentence: the run lock and layout
version check are not correctness structures and are exempt from the absence⇒downgrade
rule; they are available on Trino exactly as on DuckDB.

`docs-site/docs/guide/targets.md` — in the Trino limitations list, one line saying single-
writer locking and state versioning work normally on Trino (so the list is not read as
"nothing under `.smelt/` works here").

## Tests (red-green)

1. `crates/smelt-state/src/file_store.rs` — `stateless_lock_is_a_specified_no_op`: a
   `FileStore::with_state_mode(..., Stateless)` yields a guard with no file, a second
   acquisition also succeeds, and no `.smelt/lock` path is created. Doc comment cites the
   new spec paragraph, converting silence into a pinned claim.
2. `crates/smelt-runtime/tests/lock_backend_independence.rs` —
   `execute_project_acquires_the_state_lock_unconditionally`: structural gate over
   `crates/smelt-runtime/src/execute/project/mod.rs` asserting exactly one
   `file_store.lock()` call site and that its enclosing binding is unguarded by any
   target-, dialect- or backend-conditional (no `if`/`match` on target kind introducing
   it). Offline, per-PR — this is what keeps criterion 9 from evaporating when the tier is
   down.
3. `crates/smelt-cli/tests/trino_lock_versioning.rs` (live-gated on `SMELT_TRINO_URL`,
   skips green when unset):
   - `held_lock_refuses_a_second_trino_run_by_pid`: test process holds
     `FileStore::new(root, "trino").lock()`; `smelt run --target trino` exits non-zero with
     `state locked by PID <test pid>`; `.smelt/targets/trino/runs/` and `reports/` stay
     empty and the model's Iceberg table does not exist.
   - `releasing_the_lock_lets_the_next_trino_run_proceed`: after dropping the guard the
     same command succeeds and the table holds the expected rows — the "exactly one
     proceeds" other half.
   - `future_state_version_refuses_a_trino_run_before_any_write`: `meta.json` with
     `{"state_version": 99}` makes `smelt run --target trino` fail naming `99`, with no
     Iceberg table created.
   - `trino_lock_legs_skip_not_pass_when_url_unset`: the vacuous-pass guard, mirroring
     `trino_smoke.rs`.

## Tasks

1. Make the three spec/doc edits above.
2. Add test 1 to `file_store.rs`'s test module; confirm current behaviour matches (if it
   does not, the no-op claim is wrong and the spec paragraph must follow the code).
3. Add test 2 as a new `smelt-runtime` integration test; if the lock call site turns out
   to be reachable only under a condition, fix the call site rather than the test.
4. Add `crates/smelt-cli/tests/trino_lock_versioning.rs`, reusing
   `common::{trino_env, trino_schema, trino_target_block, fetch_trino_rows,
   drop_trino_schema}` and `trino_smoke.rs`'s `TRINO_ENV_GUARD` + subprocess-run shape.
   Stage a one-model project at `state: mode: intervals` so `.smelt/` is live.
5. Run the live legs against `scripts/trino-up.sh` + `scripts/trino-env.sh`; record the
   refusal text verbatim in the phase summary (this outcome measures, it does not read).
6. Write `phases/08-summary.md`, including the decision-log line for the stateless no-op.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-state --lib file_store`
- `cargo test -p smelt-runtime --test lock_backend_independence`
- `bash scripts/trino-up.sh && source scripts/trino-env.sh && cargo test -p smelt-cli --test trino_lock_versioning` (emit `<<PHASE_BLOCKED>>` if the coordinator is unreachable — never skip green on the live leg)
- `cargo test -p smelt-cli --test trino_spec_freshness --test trino_docs_freshness`
- `bash .claude/scripts/large-file-check.sh` (hand-edit the baseline if needed — never `--update`)

## Commit message

`feat(state): locking and versioning realised on Trino — refusal by PID proved live, and the stateless no-op stated rather than silent`
