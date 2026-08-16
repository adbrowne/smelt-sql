# Phase 2 plan — Delta discovery names affected keys

## Objective

Build `derive_affected_keys` as a pure, walk-backed, fail-closed proof in
`smelt-logical`: given a changed input's delta row shape and the model SQL, decide whether the
output group keys the delta can touch are discoverable, and name the grain columns that project
them. This is success criterion 2's third admission obligation (obligation 7) — the only new one —
and the input phase 3's per-group recompute derivation gates on. Proof only: no plan-cell
derivation, no emission, no wiring (phases 3–5).

## Spec delta

`docs/specs/model_properties.md` §Surface → "Derived proofs" table, the **Affected-key discovery**
row: status column `not-yet` → `partial (proof derived; not yet consumed by plan derivation)`.
The §Semantics text ("Affected-key discovery", landed in phase 1) is normative as written and is
**not** edited — the implementation conforms to it, including the verdict names
(`Keys{cols}` / `NotDiscoverable{reason}`), the sound-over-approximation rule, and the two named
`NotDiscoverable` causes.

## Tests

Red-green, unit tests in `crates/smelt-logical/src/analysis/affected_keys.rs` unless noted:

1. `unkeyed_retraction_is_not_discoverable` — a delta with no per-row identity yields
   `NotDiscoverable`, naming the unkeyed-retraction reason.
2. `group_by_key_over_present_delta_columns_yields_keys` — `GROUP BY customer_id` with the delta
   carrying `customer_id` yields `Keys{["customer_id"]}`.
3. `grain_expression_reading_absent_delta_column_is_not_discoverable` —
   `GROUP BY date_trunc('day', event_ts)` with a delta shape lacking `event_ts`.
4. `grain_expression_over_present_delta_column_is_discoverable` — same model, delta carries
   `event_ts`: discoverable (the expression is evaluable over the delta rows).
5. `model_with_no_proven_grain_is_not_discoverable` — an ungrouped, keyless model fails closed.
6. `declared_unique_key_supplies_the_grain_when_walk_proves_none` — a declared key in the context
   is admissible as the grain, matching `maintenance::derive::row_identity`'s precedence.
7. `delta_carrying_extra_columns_is_still_discoverable` — a superset delta shape is admitted
   (over-approximation costs work, never correctness).
8. `set_operation_root_is_not_discoverable` — the walk's fail-closed root case.
9. `opaque_expression_provenance_is_not_discoverable` — a grain column computed by a
   registry-unrecognised function over the source fails closed rather than guessing its columns.
10. `cte_composed_grain_column_resolves_through_rename_chain`
    (`crates/smelt-logical/tests/affected_keys.rs`) — a grain column reaching the source through
    a CTE rename still resolves, proving the derivation composes via the walk rather than
    scanning the model's own outermost `SELECT` text.

## Tasks

1. Add `crates/smelt-logical/src/analysis/affected_keys.rs`; register in `analysis/mod.rs`.
2. Define `AffectedKeys::{Keys{cols}, NotDiscoverable{reason}}` (`Serialize`, matching sibling
   proof verdicts) and the input shapes: `DeltaShape { source, columns, keyed }` plus an
   `AffectedKeyContext` carrying the declared `unique_key` and the `JoinContext` the walk needs.
3. Derive the grain: declared `unique_key` first, else `walk::model_property_vector(sql, ctx)`'s
   proven `Grain` key (fan-out-gated, same fail-closed rule as `derive::row_identity`); no key ⇒
   `NotDiscoverable`.
4. Resolve each grain column's source-column dependency set through the shared walk lineage —
   reuse `analysis::fingerprint`'s per-column leaf classifier by parameterising it with an
   output-column filter rather than writing a second copy; an unresolved/opaque/wildcard column
   ⇒ `NotDiscoverable{reason}`.
5. Gate: `!keyed` ⇒ `NotDiscoverable`; any required source column absent from `DeltaShape::columns`
   ⇒ `NotDiscoverable` naming the column; otherwise `Keys{cols}` = the grain columns.
6. Doc comment on the module + entry point citing `model_properties.md` §"Affected-key discovery"
   and `incremental_models.md` §"The repair family"; classify any `.contains("` introduced as a
   leaf classifier (walk-coverage gate) — prefer introducing none.
7. Apply the §Surface status-column spec edit above.

## Verification

- `bash .claude/scripts/verify-phase.sh` (export `DUCKDB_LIB_DIR=/home/andrew/.local/lib/duckdb`
  and matching `LD_LIBRARY_PATH` first — see phase 1 summary).
- `cargo test -p smelt-logical --test walk_coverage` — the property-composition-walk gate
  (success criterion 6: no new whole-text scans).
- `cargo test -p smelt-logical affected_keys` — the new unit + integration tests.

## Commit message

`feat(incremental): derive_affected_keys — fail-closed affected-key discovery for the repair family`
