# Phase 27f summary — same-partition self-read refused as non-convergent

**Shipped:**
- `docs/specs/incremental_shapes.md` §"Window independence and self-referential models": states the proof explicitly — no forward margin *and* a strictly positive backward reach; a zero-backward (current-partition) self-read is circular, refused identically to a forward read, by the same derivation `incremental_models.md` §"Time-unrolled self-edges" already cites.
- `crates/smelt-logical/src/analysis/window_independence.rs::self_edge_bound_days`: the `after == 0` bounded arm now splits on `before.0 > 0` (still `Ok`, ceiled to whole days) vs `before.0 == 0` (new `Err`, "reads only its own current partition — circular, not convergent partition-by-partition"). Doc comments on `WindowIndependence::Refused`, `window_independence`'s module doc, `self_edge_bound_days`'s contract, and `windowing.rs::compute_incremental_windows_ordered`'s `Refused` bullet updated to list the same-partition case.
- 5 new tests: 3 in `window_independence.rs` (`same_partition_self_read_is_refused_fail_closed`, `self_edge_clamp_refuses_a_same_partition_self_read_with_the_same_reason`, `sub_day_backward_self_edge_stays_ordered` — pins the check is on positive *seconds*, not ceiled days), 1 in `windowing_ordered.rs` (`same_partition_self_read_is_not_eligible_for_batched_execution`), 1 in `typed_edge_graph.rs` (`same_partition_self_edge_refuses_at_the_clamp_call_site` — `build_forward_graph` itself now refuses, not `propagate`'s later generic gate).

**Decisions:**
- Split the `after == 0` match arm on `before.0` rather than adding a leading guard clause, so both `Ok`/`Err` texts stay adjacent and the "whole days, ceiled outward" comment stays attached to the branch it describes.
- Left `propagate.rs::self_edges`'s own `before_seconds <= 0` refusal in place unchanged (it's now unreachable via the `build_forward_graph` path since the earlier `self_edge_clamp` call refuses first, but it remains the correct defensive check for any other edge-construction path, and is exercised directly by `smelt-logical`'s pure-math tests).

**For the next planner:**
- Two pre-existing fixtures encoded the *old* two-layer behavior and needed updating (both were exactly the shape this phase targets, not scope creep): `crates/smelt-cli/tests/since_upstream.rs::self_referential_node_refuses_fail_loud` asserted `stderr.contains("self-referential")` (propagate.rs's old text); now asserts `"current partition"`/`"circular"`. `crates/smelt-runtime/tests/since_upstream_propagation.rs::same_partition_self_referential_model_refuses` asserted `build_forward_graph` *succeeded* and only `plan_since_upstream` refused later; rewritten to assert `build_forward_graph` itself refuses (doc comment rewritten to match). No other fixture or `examples/` model exercised this shape.
- Row 27g (runtime dispatch for 27d's write-mechanism selection) is next per the outcome table.

**Gates:**
- `cargo test -p smelt-logical --lib analysis::window_independence` — 11 passed.
- `cargo test -p smelt-runtime --test windowing_ordered --test windowing_parity --test self_referential_ordered_backfill` — all passed.
- `cargo test -p smelt-cli --test example_diagnostics` — 119 passed, 1 ignored.
- `cargo test -p smelt-cli --test property_discovery` — 19 passed (g_08/g_11/g_13 unaffected).
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings both feature sets, full workspace `cargo test`, example_diagnostics).
