# Phase 27f — `window_independence` must require a strictly positive backward reach

## Objective

`self_edge_bound_days` admits `BoundResult::Bounded { after: 0, before: 0 }` — a
**same-partition** self-read — as `Ok(0)`, so `window_independence` returns `Ordered`
and `self_edge_clamp` returns a zero clamp. The propagation graph refuses exactly that
shape (`propagate.rs::self_edges`, `before_days <= 0`), so the two verdicts diverge
today despite `self_edge_clamp`'s doc comment promising they cannot. Require
`before > 0` in the shared derivation. Advances the outcome's end-to-end criterion by
closing the last residue phase 22 surfaced and left open (outcome Decision log,
2026-09-03 phase 22 / phase 25 planning entries).

## Spec delta

`docs/specs/incremental_shapes.md` §"Window independence and self-referential models"
(the paragraph ending "…is refused at planning time"): state the proof explicitly and
identically to `incremental_models.md` §"Time-unrolled self-edges" — an ordered model's
self-reference must read **no forward margin and a strictly positive backward reach**
over the declared partition axis. A self-read confined to the *current* partition
(`before == 0`) is circular, not convergent: the partition's own output would have to
exist before it is written. It is refused at planning time on both the ordered-backfill
execution path and the propagation graph, by the same derivation.

No Known Divergences bullet names this gap in any spec, so none is removed.

## Tests

Red-green, in this order.

`crates/smelt-logical/src/analysis/window_independence.rs` (`mod tests`):
1. `same_partition_self_read_is_refused_fail_closed` — a self-join equating the
   self-reference's partition column to the current partition yields `Refused`, with a
   reason naming the model and the same-partition (non-convergent) shape.
2. `self_edge_clamp_refuses_a_same_partition_self_read_with_the_same_reason` — the
   clamp entry point returns `Err` carrying byte-identical text to the verdict's reason
   (the existing parity test's shape, for the new arm).
3. `sub_day_backward_self_edge_stays_ordered` — a backward reach shorter than one day
   (e.g. `INTERVAL '1 hour'`) is still `Ordered` with clamp `1`, pinning that the new
   check is on positive *seconds*, not on the ceiled day count.

`crates/smelt-runtime/tests/windowing_ordered.rs`:
4. `same_partition_self_read_is_not_eligible_for_batched_execution` —
   `compute_incremental_windows_ordered` returns `Err` naming the non-convergent
   self-edge, where it previously returned silently-forced per-partition windows.

`crates/smelt-runtime/tests/self_referential_ordered_backfill.rs` (or the propagation
test file that already builds a self-edge graph — reuse, do not add a new file if one
fits):
5. `same_partition_self_edge_refuses_at_the_clamp_call_site` — the propagation graph
   build fails with `MaintenanceGraphUnsupportedNode` carrying the *derivation's* reason
   (from `propagation.rs`'s `self_edge_clamp` call site), not `self_edges`' later
   `no derivable backward bound` text — i.e. the two layers now refuse at one place.

## Tasks

1. Make the spec edit above (spec-first).
2. Add tests 1–5 red.
3. In `self_edge_bound_days`, guard the zero-forward arm with `before.0 > 0`; add an
   arm for `Bounded { after: ZERO, before: 0 }` returning a reason that names the model
   and says the self-edge reads only its own current partition — circular, not
   convergent partition-by-partition.
4. Update the doc comments that state the property — the module header, the
   `WindowIndependence::Ordered`/`Refused` variant docs, `self_edge_clamp`'s
   "can never diverge" paragraph, `self_edge_bound_days`'s `None`/`Some` contract, and
   `windowing.rs::compute_incremental_windows_ordered`'s `Refused` bullet — to list the
   same-partition case alongside forward-read / unbounded / underivable.
5. Run the **full workspace** suite, not just the listed tests: reclassifying a verdict
   is exactly the change that broke non-regression tests outside the file list in phase
   25. Fix any fixture that relied on `Ordered` for a `before == 0` self-read; if an
   `examples/` model turns out to have that shape, correct the model (its self-read was
   never executable) rather than weakening the check.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --lib analysis::window_independence`
- `cargo test -p smelt-runtime --test windowing_ordered --test windowing_parity --test self_referential_ordered_backfill --test propagation`
- `cargo test -p smelt-cli --test example_diagnostics`
- `cargo test -p smelt-cli --test property_discovery` (the self-referential probes
  `g_08`, `g_11`, `g_13` are the most likely fallout)

## Commit message

`fix(analysis): refuse a same-partition self-read as non-convergent, matching the graph layer`
