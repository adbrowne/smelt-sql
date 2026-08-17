#![cfg(feature = "bigquery")]
//! BigQuery twin of the standing generative maintenance-conformance gate
//! (`crates/smelt-cli/tests/maintenance_conformance/`;
//! `docs/plans/20260817-bigquery-generative-conformance.md` Phase 5;
//! `docs/specs/incremental_models.md` §"The equivalence invariant";
//! `docs/specs/multi_backend.md` §"Generative equivalence coverage"). Drives
//! the SAME recipe pools, schedule driver, and S-restricted multiset oracle
//! (`smelt_maintenance_testkit`) the DuckDB and Spark legs use, against a
//! live BigQuery warehouse — the backend under test is a parameter, not a
//! duplicated harness. Every `#[test]` here is a thin wrapper: staging/
//! drive/assert logic lives in `smelt_maintenance_testkit::families::*`
//! (`backend.rs` supplies only the BigQuery-specific facts — per-case
//! dataset, pacing, corruption SQL — through `ConformanceBackend`).
//!
//! Only compiled at all with `--features smelt-cli/bigquery` (this whole
//! binary is `#![cfg(feature = "bigquery")]`), so a bare `cargo test` never
//! reaches it. Each test additionally skips (green) when `SMELT_BQ_PROJECT`
//! is unset, so `cargo test --features smelt-cli/bigquery` without a live
//! project stays green too — a project set but no valid access token is
//! deliberately NOT covered by this skip: `BigQueryConformanceBackend`'s
//! `preflight_or_panic` and the live connection attempt itself both fail
//! loud in that case, never silently skip (`docs/specs/multi_backend.md`
//! §Surface's `SMELT_BQ_ACCESS_TOKEN` — no application-default-credentials
//! fallback).
//!
//! **First live run (2026-08-17, against `smelt-bq-test-20260816`): 7/21
//! passed, 14 failed.** The three backend-assumption defects this phase
//! originally found (`EXCEPT ALL`, `USING DELTA`/`open_backend`'s missing
//! `case`, `dag.rs`'s `spark`-only cfg gate) were fixed upstream before this
//! run (`docs/plans/20260817-bigquery-generative-conformance.md`) and are no
//! longer the blocker — `cargo check -p smelt-cli --features bigquery
//! --tests` now passes with `bigquery` alone. The live run surfaced THREE
//! FURTHER, DIFFERENT defects, none in this binary's own Critical files, so
//! none is fixed here:
//!
//! 1. **`smelt_maintenance_testkit::s_tracker::STracker::materialize_s_as_view`
//!    emits `SELECT * FROM (VALUES (...), (...)) AS t(cols)`.** GoogleSQL has
//!    no table-value-constructor `VALUES` clause in a `FROM` position —
//!    confirmed live: `400 Syntax error: Expected keyword JOIN but got
//!    ')'/','`. This is the S-restricted oracle's own materialization step,
//!    reached by every equivalence-asserting wrapper, so it is the single
//!    largest cause of the 14 failures (`gate` ×4, `gate_keyed`, `gate_mixed`,
//!    `pinned` ×2, and `harness_self_check_bigquery::
//!    oracle_flags_a_seeded_divergence_on_bigquery` itself — which therefore
//!    never reached its corruption step at all: this run does NOT establish
//!    the leg is non-vacuous, only that it correctly ERRORS on GoogleSQL
//!    today).
//! 2. **`smelt-runtime`'s `ephemeral_seed_ctes` compiler path (`execute.rs`/
//!    `types.rs`) also emits a `FROM (VALUES ...) AS t(cols)` construct** —
//!    this is real product code, not testkit — confirmed live via
//!    `composed_keyed_pool_upholds_equivalence_on_bigquery`'s failure, whose
//!    panic shows the actual *compiled model SQL* (`CREATE TABLE ... AS
//!    SELECT ... FROM (VALUES (900, DATE '2024-03-02', 10), ...) AS t(id, d,
//!    val) ...`), not a testkit-internal query. The same GoogleSQL gap as
//!    point 1, in a different owner.
//! 3. **`families::dags`'s two-project-per-case staging collides on
//!    BigQuery.** `families::dags::stage_dag_pair`-shaped helpers call
//!    `b.target(case)` twice (once for the "incremental" project, once for
//!    the "full-refresh" oracle twin) with the SAME `case`, so a per-case-
//!    dataset backend (BigQuery) resolves both to the identical dataset —
//!    the second `create_bigquery_source_table` call then fails `409 Already
//!    Exists`. Harmless on Spark (one persistent schema for every case
//!    regardless, and Spark's staging drops-before-seeding) but wrong for a
//!    fresh-per-case design. Blocks 4 of the 5 `dags_bigquery.rs` wrappers
//!    (`keyed_grain_node_excluded_from_generated_graph_on_bigquery` passed —
//!    it stages only one project).
//!
//! A fourth, narrower gap surfaced once, independent of the above:
//! `gate_bigquery::append_only_partition_pool_upholds_equivalence_on_bigquery`
//! hit case `recipe_holistic_agg`'s `MEDIAN(val)` aggregate, which GoogleSQL
//! does not have (`400 Function not found: MEDIAN`) — a construct the shared
//! recipe pool renders unconditionally regardless of target dialect.
//!
//! None of the above is this phase's own code: `backend.rs` and every
//! wrapper file are unchanged by this finding, and no fix was attempted
//! outside this binary's Critical files. The 7 PASSING wrappers are
//! genuinely green against the live warehouse (dataset lifecycle, admission-
//! only classification, and the boundary-row reach check, none of which
//! reach the broken code paths above).
//!
//! **Run with `-- --test-threads=1`** (mirrors the Spark twin's own
//! constraint) — every case's dataset is fresh and isolated, but the token
//! preflight budgets against ONE sweep's estimated duration per test; running
//! tests concurrently would race down the same token's remaining window
//! faster than any single test's own estimate accounts for.

mod backend;
mod dags_bigquery;
mod feed_bigquery;
mod gate_bigquery;
mod gate_composed_bigquery;
mod gate_keyed_bigquery;
mod gate_mixed_bigquery;
mod harness_self_check_bigquery;
mod pinned_bigquery;
