# Phase 1 summary — Spec delta: the `trino` target

**Shipped:**
- `docs/specs/smelt_yml.md` §"Target shape": `trino` added to the `type:` enumeration; new
  `port`/`user`/`tls`/`password` rows; `catalog`/`schema`/`host` rows extended to cover Trino;
  a `trino` refusal paragraph naming all nine refused keys; Known Divergences entry widened to
  "Databricks- and Trino-only".
- `docs/specs/multi_backend.md`: Backends bullet gains `trino`/`SqlDialect::Trino`/
  `BackendCapabilities::trino_iceberg()`; capability matrix gains a **Trino (Iceberg)** column,
  every cell `?`, with a note on how it gets measured; `SMELT_TRINO_URL` bullet; Session
  initialization, Connection security, Loading data, and Cross-engine data exchange sections
  each gain a Trino paragraph; two new Known Divergences entries (implicit-`Native` emission
  hole naming `20260913-trino-emission`, unmeasured capability column); References gains
  `crates/smelt-backend-trino/` (forward reference) and this outcome under Plans.
- `docs/specs/diagnostics.md` §Surface: one sentence stating `smelt.yml` target-shape/
  literal-secret violations are hard config-load errors with no `DiagnosticCode`, and why.
- `crates/smelt-cli/tests/trino_spec_freshness.rs` (new, 4 tests): standing drift gate mirroring
  `state_docs_freshness.rs`.

**Decisions:**
- Extended the *existing* Databricks refusal/key-placement prose in place (one paragraph per
  backend) rather than generalizing to a table, matching the plan's instruction to mirror the
  Databricks paragraph shape.
- Kept "phase 7"/"a later phase of this outcome" lowercase in spec body prose (Loading data,
  Known Divergences) — the timeless-oracle CI gate is `rg 'Phase [A-Z0-9]'` (capital P), and the
  plan explicitly calls for a forward pointer to the measurement phase; verified the grep gate
  still returns nothing new.

**For the next planner:**
- Phase 2 (`DialectId::Trino` + `SqlDialect::Trino`, exhaustiveness) is next and has no blockers
  from this phase.
- The capability matrix's Trino column is now `?` everywhere per spec; phase 8 is the one that
  turns each `?` into a measured value — don't let an earlier phase pre-fill a cell.
- Nothing found out of scope; this was a clean doc-only phase.

**Gates:**
- `cargo test -p smelt-cli --test trino_spec_freshness` — 4/4 pass.
- `cargo test -p smelt-dialect --test capability_conformance` — 2/2 pass, untouched.
- `rg -n 'Phase [A-Z0-9]' docs/specs/multi_backend.md docs/specs/smelt_yml.md docs/specs/diagnostics.md` — only pre-existing rule-statement lines matched, no new hits.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck, full workspace test, example_diagnostics).
