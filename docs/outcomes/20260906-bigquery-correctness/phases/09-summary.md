# Phase 9 summary

**Shipped:**
- `crates/smelt-maintenance-testkit/tests/googlesql_render.rs` (new) — 4 tests proving every
  `DagBody` variant (all six DAG recipes) and every `ComposedRoute`'s rendered body prints
  clean GoogleSQL, plus a non-vacuity negative control and a fail-loud check on an unparseable
  body. This is the offline gate that would have caught `diamond_propagation_suffices`'s
  infix-`%` defect before any live warehouse sweep.
- `crates/smelt-cli/tests/maintenance_conformance_bigquery/backend.rs` — new
  `oracle_relation_tests` module, `bigquery_oracle_relation_issues_no_ddl_and_returns_an_inline_subquery`,
  exercising the real `BigQueryConformanceBackend::oracle_relation` against a real
  `DuckDbBackend`. `#[cfg(feature = "duckdb")]`-gated; needs no warehouse, no
  `SMELT_BQ_PROJECT`.
- `main.rs` and `gate_composed_bigquery.rs` doc comments retired their stale
  "uncharacterised"/"not yet re-confirmed" wording, replaced with the plan's citation table
  and the 2026-08-21 (21/21) / 2026-08-22 (22 cases, concurrent) sweep results.
- `docs/specs/multi_backend.md` §"Known Divergences" — new entry "The BigQuery conformance
  leg's live evidence has a date", naming the 2026-08-22 sweep and the offline gates standing
  in between sweeps.
- `docs/handoffs/2026-09-08-github-activity-findings.md` §"Criterion 6" — the citation table,
  the offline gates, and the named HEAD-re-sweep debt.

**Decisions:**
- No fix was needed — the plan's citation table verified exactly as written against the repo
  (all 6 commits, both lowering-test suites). Phase 9's product is purely the durable/offline
  half, as the plan anticipated.
- Test 5 was written against the real `BigQueryConformanceBackend`, not a generic fake, even
  though `crates/smelt-maintenance-testkit/src/families/mod.rs` already has a generic-shape
  test (`oracle_relation_bigquery_shape_emits_no_ddl_and_returns_a_derived_table`) — the plan
  asked for coverage of the concrete type, and the generic test doesn't prove
  `BigQueryConformanceBackend`'s own override wires correctly.

**For the next planner:**
- `findings_handoff_names_no_unknown_relation` (`crates/smelt-cli/tests/github_activity_oracle.rs`)
  scans the whole handoff document for ANY markdown table row starting `` | ` `` and treats it
  as a registered-divergence claim — not scoped to the actual divergence table. This phase's
  new §"Criterion 6" table tripped it and had to be reformatted (`` | Test: `name` ``) to avoid
  the false positive. If a future handoff edit adds another backtick-leading table, expect the
  same trip; the scan itself could be tightened to the specific section, but that's out of
  this phase's scope.
- Row 10 (Close) is next: regenerate `docs/reference/dialect-coverage.md`, move gap ratchets
  down, update issue #179, confirm all standing gates green.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN
- `cargo test -p smelt-maintenance-testkit --test googlesql_render` — 4/4 passed
- `cargo test -p smelt-dialect --test modulo_lowering --test power_lowering` — 11/11 passed
- `cargo test -p smelt-cli --test maintenance_conformance --features duckdb` — 101/101 passed
- `cargo check -p smelt-cli --features bigquery --tests` — clean
- `cargo test -p smelt-cli --features bigquery --test maintenance_conformance_bigquery bigquery_oracle_relation` (`SMELT_BQ_PROJECT` unset) — 1/1 passed, ran (not skipped)
- `bash .claude/scripts/large-file-check.sh` — OK
