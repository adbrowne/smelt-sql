# Phase 8 summary — Locking and versioning: realised, not silently absent

## Shipped

- `docs/specs/run_state.md` §"Locking" — new paragraph stating the run lock and layout-version
  check are backend-independent, fire before any state write, and that `state.mode: stateless`'s
  `lock()` is a specified no-op (absence of state, not absence of a lock).
- `docs/specs/state.md` §"The degradation contract" — one sentence exempting the lock/version
  check from the absence⇒downgrade rule (not a correctness structure).
- `docs-site/docs/guide/targets.md` — Trino limitations section now states locking/versioning
  work normally, so the `✗`-heavy list isn't misread as "nothing under `.smelt/` works here".
- `crates/smelt-state/src/file_store.rs` — `stateless_lock_is_a_specified_no_op` (unit test):
  two concurrent stateless lock acquisitions both succeed, neither creates `.smelt/lock` or
  `.smelt/`.
- `crates/smelt-runtime/tests/lock_backend_independence.rs` — structural gate: exactly one
  `file_store.lock()` call site in `execute/project/mod.rs`, anchored on its `.context(...)`
  string; the 5 lines immediately preceding it contain no `if`/`match`.
- `crates/smelt-cli/tests/trino_lock_versioning.rs` — 4 live-gated tests against a real Trino
  coordinator: held-lock refusal names the holder's PID and leaves `.smelt/targets/trino/{runs,
  reports}` empty and the Iceberg table absent; releasing the lock lets the next run succeed and
  materialize the table; a `meta.json` with `state_version: 99` refuses before any write; the
  vacuous-pass guard for `SMELT_TRINO_URL` unset.
- `.claude/large-file-baseline.txt` — `file_store.rs` bumped 1695 → 1723 (one new test).

## Decisions

- No production code changed. `FileStore::lock()` (`file_store.rs:405`) and its single call site
  in `execute_project` (`execute/project/mod.rs:862`) were already unconditional and already
  no-op under `stateless` — this phase converts that fact from an implicit property into a
  spec-pinned, test-enforced claim, per the outcome's own decision log ("the phase's work is
  demonstration ... plus making the one genuine no-op a stated behaviour").
- The structural gate anchors on the exact `.context("failed to acquire the .smelt/ state lock")`
  string rather than a looser `file_store.lock()` substring match, since the file has ~40 other
  unrelated `.lock()` calls (`graph.lock()`, `db.lock()`, tokio mutexes) that a looser match would
  miscount.
- Used `-p smelt-core --test trino_docs_freshness` (not `-p smelt-cli` as the plan's Verification
  section named it) — the plan had a stale crate name; the actual file lives in
  `crates/smelt-core/tests/`, confirmed by grepping prior phase summaries in this outcome.

## For the next planner

- Phase 8 required zero implementation work — every property it tests was already true. Worth a
  quick sanity check on phase 9's premise (`.smelt/` not correctness-bearing: delete-between-runs
  equality) before writing its plan, in case it's similarly already-true and the phase is mostly
  a proof/spec exercise rather than new code.
- Not done here, out of scope: no diagnostic code exists for the lock-contention failure (the
  decision log already notes this is intentional — it's a plain `anyhow` error, not a
  `DiagnosticCode`, since it's a process-level refusal, not a project-analysis diagnostic).

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full workspace `cargo test`, example_diagnostics).
- `cargo test -p smelt-state --lib file_store` — pass (includes new test).
- `cargo test -p smelt-runtime --test lock_backend_independence` — 2/2 pass.
- `bash scripts/trino-up.sh && source scripts/trino-env.sh && cargo test -p smelt-cli --test trino_lock_versioning` — 4/4 pass against a live coordinator; `bash scripts/trino-down.sh` after.
- `cargo test -p smelt-cli --test trino_spec_freshness` — 5/5 pass.
- `cargo test -p smelt-core --test trino_docs_freshness` — 6/6 pass.
- `bash .claude/scripts/large-file-check.sh` — OK after baseline bump.
