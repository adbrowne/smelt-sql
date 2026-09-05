# Phase 6 summary — close the bullets, sweep the residue, prove the gates

**Shipped:**
- `docs/specs/incremental_models.md` §Known Divergences "Conditional-maintenance gaps" rewritten:
  names `supports_fingerprint_sidecar` as read by every sidecar consumer (never re-derived from
  dialect), states both legs' distinct fallback (widened scan vs. `UnsupportedOnBackend`), and
  narrows the residual divergence to DuckDB alone declaring the flag today (DDL-owner-shaped).
- `docs-site/docs/guide/incremental-models.md` fingerprint-sidecar paragraph (~line 839)
  rewritten in user voice with the same two-leg framing, dropping the stale "DuckDB-only today
  … Spark or BigQuery" single-leg sentence.

**Decisions:**
- Confirmed via `residue_grep` that no production or spec/doc site still gates the fingerprint
  sidecar on `SqlDialect::DuckDB`; the remaining `dialect == /!= SqlDialect::DuckDB` sites in
  `maintenance_driver.rs` (526/1956/3486/3609 area, plus 651/677) are the ledger and
  observed-delta bookkeeping gates phase 5 explicitly named as out of scope — unrelated
  substrates, correctly DuckDB-shaped today.
- Confirmed the three `docs/TODO.md` bullets ("Frozen-horizon append-only gate", "Deferral
  oracle restatement", "Sidecar per-consuming-edge audit") were already removed by phases
  1/2/4 — nothing to do here.
- Did a targeted (not full) `/smelt:validate`-shaped check scoped to the five closed bullets per
  the plan's instruction to read drift "only for the five closed bullets": grepped each bullet's
  claimed artifact (`ContractFrozenHorizonInvalid` diagnostic + catalogue entry,
  `ExactOverProcessedSWithLagBound` oracle obligation, `consumer_address` sidecar PK column,
  capability-flag gating) and found each present and consistent with spec wording. No drift
  found on the five bullets.
- Confirmed no timeless-oracle phase-vocabulary leakage in either edited passage.

**For the next planner:**
- Phase 3's blocked clause (once-write generative-pool DuckDB witness) remains open; a human
  call is still needed among the three options in `phases/03-summary.md` before it can proceed.
  This is the only reason the outcome cannot yet be marked `done`.
- No other residue surfaced. A future outcome could still widen once-write nullability to
  key-derived expressions or build orphaned-partition GC for the fingerprint sidecar, both
  already recorded under this outcome's "Out of scope".

**Gates:**
- `bash .claude/scripts/verify-phase.sh` exceeded the foreground timeout again (as it did in
  phase 5); ran its four legs separately per the plan's fallback, all green:
  - `cargo fmt --all -- --check` — PASS (no output)
  - `bash .claude/scripts/clippy-gate.sh` — PASS (both feature sets, zero warnings)
  - `cargo test --quiet` — PASS (all suites `0 failed`)
  - `cargo test -p smelt-cli --test example_diagnostics --quiet` — PASS (120 passed, 1 ignored)
- `cargo test -p smelt-cli --test maintenance_conformance --quiet` — PASS (79 passed)
- `cargo test -p smelt-runtime --test statement_parity --test fingerprint_sidecar --quiet` —
  PASS (17 + 37 passed)

**Criterion status:** 1, 2, 4, 5 closed; 3 closed except the blocked generative-pool clause; 6
evidenced (targeted validate scoped to the five bullets found no drift; all listed gates green).
Recommend: the outcome cannot yet flip to `done` — phase 3's blocked clause needs a human
decision among its three options first.
