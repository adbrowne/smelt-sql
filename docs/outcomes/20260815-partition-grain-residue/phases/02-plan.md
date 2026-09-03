# Phase 2 plan — function-registry-threaded classification

## Objective

Make the bound-`NotDerivable` refusal gate and the window-function batch-safety check classify
the SQL that actually executes: `smelt.define` bodies expanded, and window frames visible inside
the derived tables/CTEs that expansion produces. Advances success criterion 2 and inverts the
two phase-1 probes (`probe_lookback_gate_sees_define_body`,
`probe_batch_safety_sees_over_in_define_body`).

Two halves, both required — expansion alone is not enough: a table-position call expands to a
derived table, and `analyze_one_select` never descends, so a bare `LAG`/`LEAD`/default-frame
`OVER` inside a define body would still classify `FullyBatchSafe` (an *unsafe* verdict, worse
than a conservative one). Only the `RANGE BETWEEN INTERVAL` text scan catches today's shape.

## Spec delta

`docs/specs/incremental_shapes.md` — edit first:
- §"The partition grain" → Known Divergences: delete both bullets ("One classification call site
  reads the outer SQL body", "The window-function batch-safety check runs on unexpanded outer
  SQL") and their `20260530-thread-fn-registry-classification.md` citations.
- §"Functions inside partition-grain bodies": add one sentence stating that window/frame
  classification descends through derived tables and CTE bodies, so a frame introduced by
  expansion is seen exactly as an inline one is.

## Tests

Red-green, in this order:

1. `crates/smelt-logical/src/analysis/temporal.rs` (unit) `derived_table_window_is_seen` — a
   default-frame `OVER`/`LAG` inside a FROM-clause derived table yields the same
   `TemporalDependency` as the same window written at the outer level (today: empty).
2. same file, `cte_body_window_is_seen` — `WITH w AS (SELECT ... OVER (... RANGE BETWEEN
   INTERVAL '7 day' PRECEDING ...)) SELECT * FROM w` is classified from the AST, asserted via a
   frame shape the whole-text `RANGE BETWEEN INTERVAL` scan cannot produce (e.g. `ROWS BETWEEN
   3 PRECEDING`, or a `LAG(x, 1) OVER (ORDER BY d)`).
3. `crates/smelt-runtime/tests/classification_expansion.rs` (new)
   `bound_derivation_sees_define_body_lookback` — `build_model_graph` with a `FnBodyMap`
   containing the function produces the *same* `derive_model_source_bounds` result as the
   hand-inlined query (the phase-1 probe's `assert_ne!`, inverted, at the layer that owns
   expansion).
4. same file, `batch_safety_sees_define_body_window` — a model whose only lookback lives in a
   define body classifies `BoundedSafe` (not `FullyBatchSafe`) once the graph is built with the
   fn-body map.
5. same file, `no_fn_bodies_is_identity` — an empty `FnBodyMap` leaves classification unchanged
   (no regression for projects without `smelt.define`).
6. `crates/smelt-runtime/tests/classification_expansion.rs`
   `every_production_model_info_uses_expanded_sql` — structural gate: no production source in
   `crates/smelt-{runtime,cli,ui}/src` constructs `ModelInfo { … sql: <raw model content> … }`;
   the `sql:` field must be fed by `expand_function_calls`. Fails naming the offending file:line.
7. Retarget the two phase-1 probes: delete `probe_lookback_gate_sees_define_body` and
   `probe_batch_safety_sees_over_in_define_body` from
   `crates/smelt-logical/tests/partition_residue_probes.rs` (their subject moved to the runtime
   layer, tests 3–4 above); leave the other two probes untouched and note the move in the
   file's module doc.

## Tasks

1. Spec edit above (spec-first).
2. Add nested-select descent to `analyze_temporal_dependencies`: walk FROM-clause derived tables
   and CTE bodies, calling `analyze_one_select` at each level; merge via the existing `max_with`.
   Update `analyze_one_select`'s "does not descend" doc comment, and classify the surviving
   `analyze_subquery_range_frames` text scan in its doc comment per CLAUDE.md's property-
   composition-walk rule (leaf classifier / advisory belt-and-braces — it must not be the only
   producer of a bound any more).
3. `crates/smelt-runtime/src/safety.rs`: `build_model_graph` takes `fn_bodies: &FnBodyMap` and
   stores `expand_function_calls(&model.content, fn_bodies)` in `ModelInfo.sql`.
4. `crates/smelt-runtime/src/execute.rs:607`: pass the `fn_bodies` already built at line 563.
   Update `crates/smelt-runtime/tests/safety_check_parity.rs` call sites.
5. `crates/smelt-ui/src/build.rs:~175` and `~529`: expand before `analyze_batch_safety`.
   `build_model_details` has a `db`; `build_run_plan` needs the map threaded from `api.rs:125`
   (build once via `build_fn_body_map`, don't rebuild per model).
6. Run the examples: a model whose classification *changes* under descent is a true positive
   (a real hidden window) unless the window is provably irrelevant to the grain — if any
   example now refuses, record which and why in the phase summary; fix the classifier only if
   the verdict is wrong, never by narrowing the descent to make examples pass.
7. Write `phases/02-summary.md` (findings, any classification changes, gates).

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt, clippy both feature sets, workspace tests,
  example_diagnostics) — must be all green.
- `cargo test -p smelt-logical --test walk_coverage` — the descent must not violate the
  property-composition-walk rule.
- `cargo test -p smelt-runtime --test classification_expansion` (new).
- `cargo test -p smelt-logical --test partition_residue_probes` — remaining 2 probes still pass.
- `cargo test -p smelt-cli --test partition_residue_probes` — 3 probes unaffected.

## Commit message

`feat(logical): classify partition-grain models through expanded fn bodies and nested selects`
