# Phase 1 summary — posture confirmed by measurement

## Shipped

- `scripts/trino-probe-state.sh` — speaks Trino's `/v1/statement` protocol directly (curl + jq),
  threading `X-Trino-Started-Transaction-Id` forward as `X-Trino-Transaction-Id` so the probe
  genuinely holds one transaction session, unlike smelt's stateless backend client. Runs cases
  A–G from the plan (baseline, bare START/COMMIT, cross-table happy path, cross-table with a
  failing second statement, explicit rollback, DDL-in-transaction, same-table two writes), each on
  fresh per-run tables, each verdict confirmed by an out-of-transaction row count rather than a
  trusted statement-success flag.
- `crates/smelt-core/tests/trino_docs_freshness.rs` — two new assertions:
  `probe_state_script_exists_and_is_executable`, `readme_documents_the_state_probe`.
- `scripts/README-trino.md` — new "Measuring the state-residency posture" section.
- `docs/outcomes/20260913-trino-ledger/outcome.md` — dated decision-log entry with the full
  measured table and verbatim server error text; phase-1 row flipped to `done`.

## Decisions

- **Every request must carry `X-Trino-Transaction-Id`, `NONE` when no transaction is open.**
  Omitting the header entirely (rather than sending `NONE`) makes Trino refuse `START TRANSACTION`
  with `Client does not support transactions` — discovered live, not documented anywhere read
  beforehand. See decision log for the full reasoning.
- **The probe creates its own schema.** `smelt_dev` does not pre-exist on a fresh tier; the probe
  runs `CREATE SCHEMA IF NOT EXISTS iceberg.smelt_dev` up front, matching what
  `crates/smelt-backend-trino/src/backend.rs:315` already does for real runs.
- **No escalation.** The measured verdict — Iceberg refuses every write (DDL or DML, same-table or
  cross-table) inside an explicit transaction with `Catalog only supports writes using autocommit:
  iceberg` — is *stronger* than "no cross-table atomicity" but is still exactly the shape phases
  2–5 assume (claims none of the five correctness structures). Criteria 2–5 proceed unchanged.

## For the next planner

- **Phase 3 (schema-evolution DDL) should re-confirm `supports_transactional_ddl = false`
  independently** — this phase's case F answers the same cell but through DDL run *inside* an
  explicit transaction, which is a different code path from T1's plain autocommit DDL probe. Both
  now agree, but phase 3 measures the DDL forms themselves (ADD COLUMN, widen, etc.), not just the
  transactional-wrapper question.
- **`START TRANSACTION`/`COMMIT`/`ROLLBACK` are real, write-inert syntax on Trino/Iceberg** — they
  succeed but every write inside one is refused. This could matter for a future read-consistency
  feature (a multi-statement read snapshot) but is out of scope here; noted for anyone who later
  wonders why the SQL surface has transaction keywords despite "no correctness structures."
- **The autocommit-refusal error text (`Catalog only supports writes using autocommit: iceberg`)
  is worth keeping as the exact string phase 4/5's error-path tests assert against**, if any test
  needs to distinguish "Trino refused this write for the transaction reason" from a generic
  failure.
- Nothing else came up out of scope; the phase shipped exactly what criterion 1 and the plan
  specified.

## Gates

- `cargo test -p smelt-core --test trino_docs_freshness` — 6 passed (4 existing + 2 new).
- `mise exec -- shellcheck scripts/trino-probe-state.sh` — clean, no findings.
- `bash scripts/trino-up.sh && source scripts/trino-env.sh && bash scripts/trino-probe-state.sh`
  — ran against the live tier; every case printed a verdict (see decision log for the full table).
- `bash .claude/scripts/verify-phase.sh` — run below; see commit for result.
- No baseline file bumped.
