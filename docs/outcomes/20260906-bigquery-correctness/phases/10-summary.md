# Phase 10 summary — Close

**Shipped:**
- `crates/smelt-logical/tests/maintenance_dialect_blindness.rs` (new) — scans every `.rs`
  under `crates/smelt-logical/src/maintenance/`, with `#[cfg(test)]` module bodies stripped,
  for a bare `MaintenanceDialect::DuckDb` outside a real `match`/`matches!` dispatch. Closes
  the last gap in criterion 3: previously the phase 1-3 fix class was held only by
  per-emitter unit tests.
- `crates/smelt-cli/tests/github_activity_oracle.rs`: `handoff_claimed_relations()` is now
  scoped to the `## The registered divergences` section (via a new pure
  `claimed_relations_in(text)`), fixing the false-positive risk phase 9's summary flagged —
  a backtick-leading row in an unrelated section (e.g. the `## Criterion 6` test-name table)
  is no longer read as a stale registered-divergence claim.
- `docs/handoffs/2026-09-08-github-activity-findings.md` gained a `## Close-out (2026-09-08)`
  section: one row per success criterion naming its artifact and holding gate, plus what
  stayed deliberately unverified (BigQuery value leg, the 42 no-verdict registry entries).
- `.claude/dialect-gaps-baseline.txt` gained a dated note: `dialect_gaps_bigquery` holds at
  42 because this outcome's fixes were all in `smelt-logical`, never `BuiltinRegistry`.
- `docs/reference/dialect-coverage.md` reconfirmed byte-identical after
  `SMELT_REGEN_DOCS=1 cargo test -p smelt-db --test dialect_audit
  the_coverage_table_matches_the_registry` (`git status` clean).
- Posted a comment on issue #179 (not closed) naming what this outcome fixed nearby and
  that its 42 entries are untouched.

**Decisions:**
- Held, not lowered, `dialect_gaps_bigquery` — no phase 1-9 fix touched `BuiltinRegistry`,
  and building any of the 42 speculatively is forbidden by criterion 2. Also appended to
  the outcome's Decision log.
- The offender classifier treats a `MaintenanceDialect::DuckDb` mention inside a string
  literal (e.g. an assert! message naming the variant) as inert, not a hardcode — needed to
  keep test 1 green at HEAD against `succession.rs`'s two `assert!` messages.

**For the next planner:**
- Nothing deferred from this phase's own scope. The outcome-level Out of scope items (the
  42 registry entries, Snowflake/Redshift/Postgres, live BigQuery value leg) remain exactly
  as recorded — this phase changed none of them, only made their status legible.
- This was the outcome's last row; `docs/outcomes/20260906-bigquery-correctness` is now
  fully `done`.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-logical --test maintenance_dialect_blindness` — 3/3 passed.
- `cargo test -p smelt-cli --test github_activity_oracle` — 18 passed, 1 ignored (measurement
  sweep).
- `cargo test -p smelt-db --test dialect_audit` — 61/61 passed.
- `cargo test -p smelt-dialect --test emission_ownership` — 11/11 passed.
- `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance` —
  18/18 + 4/4 passed.
- `cargo test -p smelt-maintenance-testkit --test googlesql_render` — 4/4 passed.
- `bash .claude/scripts/large-file-check.sh` — OK, no ratchet regression.
