# Phase 4 plan — sidecar per-consuming-edge audit, and the fix it forces

**Advances:** success criterion 4 (two consumers of one source under a shared
projection-identity partition; each consuming edge gets its own comparandum; the
`sources.md` divergence bullet rewritten to the residual gap) and, for the bullet text,
criterion 6.

## Objective

Stage two consumers of one `mutable_snapshot` source whose P4 projections are byte-identical
and audit whether the built sidecar upholds the per-consuming-edge comparandum requirement.
It does not: the sidecar row is keyed `(source_address, projection_identity, source_key)` with
no consumer discriminator, so today the only thing separating two consumers is the `stamp`'s
model-SQL hash — which fails **unsoundly** when two consuming models have byte-identical
bodies (consumer A's refresh makes consumer B's next diff report "no change" for a delta B
never consumed, exactly the hazard §"Naming and namespace" names), and fails **uselessly**
otherwise (distinct bodies mutually invalidate, so neither consumer ever gets a narrowed
delta and every run logs a spurious stamp-mismatch warning). Fix by making the consuming
model's address part of the sidecar namespace.

## Design decision (log it)

Take the explicit route: add `consumer_address` as a fourth namespace/PK column on
`_smelt_fingerprint_sidecar`, rather than folding the consumer into the `stamp`. Folding into
the stamp would fix the unsoundness but keep the thrash (each consumer's refresh still
clobbers the sibling's digest rows, so no consumer ever narrows). A distinct partition per
consuming edge is what "each consuming edge gets its own comparandum" means physically, is
directly assertable from stored rows, and removes both failure modes. The `stamp` keeps its
model-SQL hash — it still catches a definition edit *within* one consuming edge.

## Spec delta (first, before code)

`docs/specs/sources.md` §"The fingerprint sidecar" — "Naming and namespace":
- Row key becomes `(source address, projection identity, consuming model address, source key)`.
- Rewrite the "two consumers … may **share** one sidecar partition's digests" paragraph: sharing
  digests across consuming edges is deliberately **not** taken — it would require per-consumer
  consumption cursors to stay sound, and the storage saving does not justify the mechanism; the
  cursor design is recorded as the option if per-edge duplication ever becomes a cost. Keep the
  reason (a shared refresh would make a sibling's next diff silently report "no change") as the
  justification for the split, and keep the fail-closed-full-row-is-its-own-identity sentence.
- §Known Divergences: rewrite the "Whether the built sidecar upholds the per-consuming-edge
  comparandum requirement … is unverified" bullet to the residual gap — verified and enforced by
  construction per consuming edge; what remains is orphaned-partition GC (already its own bullet)
  and the un-taken shared-digest-with-cursors optimization.

## Tests (red first)

1. `crates/smelt-runtime/tests/fingerprint_sidecar.rs::two_consumers_with_identical_bodies_do_not_share_a_comparandum`
   — two consuming edges, same projection, **byte-identical** `model_sql`; A diffs+refreshes,
   the source is edited, B diffs → B must see the edited key. Red today (B sees an empty set).
2. `…::each_consuming_edge_keeps_its_own_comparandum_across_interleaved_runs`
   — A and B (distinct bodies, same projection) alternate diff+refresh over a sequence of edits;
   each consumer's changed-key set must be exactly the keys edited since *that consumer's* last
   refresh — not the whole table, and not a sibling-narrowed set.
3. `…::a_sibling_consumers_refresh_leaves_this_edges_sidecar_rows_untouched`
   — assert on stored rows: after both consumers refresh, the same source key holds one row per
   `consumer_address`, and B's refresh does not alter A's digest/stamp.
4. `crates/smelt-state/src/ddl_duckdb.rs` unit tests — DDL PK includes `consumer_address`; refresh
   `ON CONFLICT` target, stale-check, partition-exists and GC predicates all carry it (extend the
   existing five tests rather than adding parallel ones).
5. `crates/smelt-logical/src/maintenance/emit.rs::fingerprint_sidecar_tests` — the diff query's
   sidecar-side predicate filters `consumer_address` alongside source address and projection
   identity.
6. `crates/smelt-runtime/tests/statement_parity.rs` — the fingerprint-sidecar leg's expected
   diff/refresh/GC SQL updated to the new namespace (executed-vs-emitted parity must still hold).

## Tasks

1. Edit `docs/specs/sources.md` per the spec delta above.
2. Add `consumer_address` to the sidecar table DDL (column + PK) and to the refresh, stale-check,
   partition-exists and GC statement builders in `crates/smelt-state/src/ddl_duckdb.rs`.
3. Thread a `consumer_address` parameter through `emit_fingerprint_sidecar_diff` (and the shared
   `FULL OUTER JOIN` helper) in `crates/smelt-logical/src/maintenance/emit.rs`.
4. Thread it through `smelt-runtime`'s four sidecar entry points —
   `diff_fingerprint_sidecar_changed_keys`, `refresh_fingerprint_sidecar`,
   `diff_repair_group_sidecar_changed_keys`, `refresh_repair_group_sidecar` — and through
   `Backend::execute_write_and_refresh_fingerprint_sidecar` (`smelt-backend`,
   `smelt-backend-duckdb`).
5. Supply the real consuming model's address at the `crates/smelt-runtime/src/execute.rs` and
   `maintenance_driver.rs` call sites (each already threads `model_sql`; the address is in hand at
   the same point — do not synthesize one from the SQL).
6. Write tests 1–3 red, then confirm green; extend tests 4–6.
7. Update the sidecar doc comments in `ddl_duckdb.rs`/`maintenance_driver.rs` that state the
   three-part namespace, and the `sources.md` §References line if it names the key shape.

## Verification

- `cargo test -p smelt-runtime --test fingerprint_sidecar`
- `cargo test -p smelt-runtime --test statement_parity`
- `cargo test -p smelt-state --lib ddl_duckdb::`
- `cargo test -p smelt-logical --lib maintenance::emit::fingerprint_sidecar_tests`
- `cargo test -p smelt-runtime --test external_source_delta_restriction --test delta_restricted_recompute`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `bash .claude/scripts/verify-phase.sh`

## Commit message

`fix(sidecar): namespace the fingerprint sidecar per consuming edge`
