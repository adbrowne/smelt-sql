#![cfg(feature = "duckdb")]
//! Property-discovery research loop — Link-C in-process test target.
//!
//! `EXPERIMENTAL(property-discovery): disposable`. See
//! `docs/research/20260705-property-discovery-loop.md` (design) and
//! `docs/plans/20260705-property-discovery-loop.md` (plan). This target hosts
//! one module per resolved catalog cell whose construct has no equivalent in
//! `smelt_maintenance_testkit::recipe`'s typed `ModelRecipe` vocabulary — the
//! cells the standing `maintenance_conformance` gate
//! (`cargo test -p smelt-cli --test maintenance_conformance`) does not
//! subsume. Cells the gate DOES subsume (additive/idempotent/holistic
//! aggregation, redelivery, mutable-dimension enrichment, composite-key join
//! fan-out, keyed reprocessed-window refusal) were graduated into that
//! target's `pinned` module and retired from here — see
//! `crates/smelt-cli/tests/maintenance_conformance/pinned.rs`'s module doc
//! comment for the retired-probe → pinned-case mapping.
//!
//! Each remaining probe is built on the in-process real-planner harness
//! (`smelt_maintenance_testkit::link_c_harness`), a local model-SQL shape
//! (`crate::shapes`, this crate's own catalogue for the constructs the typed
//! recipe generator doesn't cover — self-referential models, `UNION ALL`,
//! `LEFT JOIN`, correlated `EXISTS`, stacked window frames, cross-source
//! column-name collision, a mutable source aggregated directly), and the
//! Link-C oracle (`smelt_maintenance_testkit::oracle`) — see
//! `docs/specs/maintenance_plan.md` §References → Tests.

mod coverage_matrix_gaps;
mod g_04_idempotent_min_mutable_snapshot;
mod g_06_left_join_null_preservation;
mod g_08_running_total_self_ref;
mod g_09_union_all_append_only;
mod g_11_self_ref_ambiguous_column;
mod g_13_self_ref_derived_output_window;
mod sc_1_correlated_exists;
mod sc_1b_column_name_collision;
mod sc_2_clocked_mutable_window_forward;
mod sc_4_stacked_frames;
mod sc_6_fd_over_union;
mod sc_7_cte_body_admission;
mod shapes;

// tracer_evolution/tracer_maintenance/tracer_propagation moved to
// `smelt-runtime/tests/` — they only depend on `duckdb` +
// `smelt_logical::maintenance::*`, not on this crate's `link_c_harness`
// (see `docs/research/20260705-refresh-as-maintenance-plan/08-code-placement.md`).
