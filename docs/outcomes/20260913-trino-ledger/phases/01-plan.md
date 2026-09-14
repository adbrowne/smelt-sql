# Phase 1 — Confirm the posture by measurement: `scripts/trino-probe-state.sh`

**Outcome:** `docs/outcomes/20260913-trino-ledger/outcome.md`
**Advances:** success criterion 1 (posture confirmed by measurement, then stated); pre-condition
for criteria 2–5, which all assume Spark's answer.

## Objective

Establish, against the live Iceberg tier rather than from documentation, whether Trino gives
cross-table atomicity — the one fact this outcome cannot inherit from Spark, because Trino has
explicit `START TRANSACTION`/`COMMIT` syntax and the answer is therefore a property of the
Iceberg connector. Land `scripts/trino-probe-state.sh` in the shape of `scripts/spark-probe-ddl.sh`
(fresh tables per case, server's own words printed verbatim) and record the verdict plus the
quoted error text in the outcome's decision log. Ship no production code: this phase measures.

## Spec delta

None. Phase 2 owns the `state.md` / `multi_backend.md` edits; this phase produces the measurement
those edits cite. If the probe finds **genuine cross-table atomicity**, do not absorb it — flip
row 2 of the phase table to `blocked`, write the finding into `## Blocked`, and escalate, since
that answer invalidates criteria 2–5 as written.

## The measurement trap this phase must avoid

T1 measured `supports_transactional_ddl = false`, and the hand-forward (decision log, 2026-09-14)
records that this measured **smelt's stateless `/v1/statement` client** — no session continuity
across `START TRANSACTION`/DDL/`ROLLBACK` — not a Trino grammar rejection. A probe written on
smelt's backend client would therefore re-measure the client and prove nothing about Iceberg.
The probe must carry the transaction across statements: `POST /v1/statement` with
`X-Trino-User`/`X-Trino-Catalog`/`X-Trino-Schema`, read `X-Trino-Started-Transaction-Id` off the
response headers, and send it back as `X-Trino-Transaction-Id` on every following statement,
following `nextUri` to drain each query. `curl` + `jq` are both present; if the header protocol
proves unworkable, fall back to the CLI inside the coordinator (`docker exec -i
smelt-trino-coordinator trino --catalog iceberg --schema smelt_dev -f -`), which holds one
session, and say in the script header which client was used and why.

## Tests

1. `trino_docs_freshness::probe_state_script_exists_and_is_executable` — `scripts/trino-probe-state.sh`
   resolves on disk and is `+x`, alongside the existing `scripts/trino-*` path assertions.
2. `trino_docs_freshness::readme_documents_the_state_probe` — `scripts/README-trino.md` names the
   probe script and the command that runs it, so the measured-not-read discipline is discoverable.
3. The probe itself is the phase's real oracle and is run by hand (below), not from `cargo test`:
   it mutates a live catalog and has no deterministic pass/fail.

## Tasks

1. Red: add the two `trino_docs_freshness` assertions; watch them fail.
2. Write `scripts/trino-probe-state.sh` — `set -euo pipefail`, require `SMELT_TRINO_URL`
   (exit non-zero with the `source scripts/trino-env.sh` hint when unset, exactly as
   `spark-probe-ddl.sh` does for `SPARK_CONNECT_URL`), a `run_stmt` helper implementing the
   statement protocol with transaction-id continuity, and a `probe` helper that creates fresh
   per-case tables under `SMELT_TRINO_SCHEMA` with a run-unique suffix and drops them in a trap.
3. Implement the candidate list, each printing `ACCEPTED`/`REFUSED` plus the server's first error
   line verbatim, and each asserting *observed row counts* after the fact rather than trusting the
   statement's own success:
   - **A. baseline** — single-statement `INSERT` outside any transaction; rows land.
   - **B. syntax** — `START TRANSACTION` then `COMMIT` with nothing between.
   - **C. cross-table happy path** — `START TRANSACTION`; `INSERT` into `t1`; `INSERT` into `t2`;
     `COMMIT`. Report which statement is refused, if any, and both tables' row counts after.
   - **D. cross-table with a failing second statement** — same, but the second `INSERT` fails
     (type mismatch), then `COMMIT`. **The atomicity question**: does `t1`'s row survive?
   - **E. explicit rollback** — `START TRANSACTION`; `INSERT` into `t1`; `ROLLBACK`; is `t1` empty?
   - **F. DDL in a transaction** — `START TRANSACTION`; `CREATE TABLE`; `ROLLBACK`; does the table
     survive? (the cell T1 measured through the wrong client).
   - **G. same-table two writes** — `START TRANSACTION`; two `INSERT`s into one table; `COMMIT` —
     separates "no cross-*table* transaction" from "no multi-write transaction at all".
4. Bring the tier up (`bash scripts/trino-up.sh`; it is currently down), `source scripts/trino-env.sh`,
   run the probe, capture the full output.
5. Append a dated decision-log entry to `outcome.md`: the verdict (expected: Iceberg refuses
   multi-write transactions, Spark's shape confirmed), the verbatim error text for C/D/E/F/G, and
   an explicit statement of whether criteria 2–5's assumption holds.
6. Document the probe in `scripts/README-trino.md` under the tier's run section (one short block).
7. Write `phases/01-summary.md` with the measured table and anything phases 3–8 must act on
   (notably F's honest answer for `supports_transactional_ddl`, and G's bearing on phase 7's
   staged relation group).
8. Tear the tier down (`bash scripts/trino-down.sh`).

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt, both clippy feature sets, shellcheck — the new
  script must be clean at `warning` with no ratchet — tests, example diagnostics).
- `cargo test -p smelt-core --test trino_docs_freshness` — the two new assertions green.
- `bash scripts/trino-up.sh && source scripts/trino-env.sh && bash scripts/trino-probe-state.sh`
  — every case must print a verdict. **If the coordinator is unreachable, emit
  `<<PHASE_BLOCKED>>`; never record an unmeasured verdict and never skip green.**
- No baseline file bumped (`.claude/hardening-baseline.txt`, `.claude/large-file-baseline.txt`).

## Commit message

`probe(trino): measure Iceberg cross-table transaction atomicity against the live tier`
