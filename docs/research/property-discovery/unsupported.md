# Unsupported combinations (the admission-matrix negative space)

Every REFUTED or CONDITIONAL cell surfaces here as one line — the catalogue of
`(construct × source-property × technique)` combinations that do **not** support a technique,
annotated with **why** (the witness schedule, the missing algebraic property, or the named guarantee
a CONDITIONAL cell trades). This is the directly reusable input for the spec admission matrices
(`keyed_models.md`, `batched_models.md`, `model_maintenance.md`).

Full detail (witnesses, evidence, smelt-analyzer verdict) lives per-cell in `ledger.md`; this file
is the scannable index.

Format:

```
<construct> × <source> — technique <T>: UNSUPPORTED — <why: witness | missing P | traded guarantee>
```

---

<!-- The loop appends one line per REFUTED/CONDITIONAL cell below this line. -->
join fan-out on composite unique key (e.g. `(user_id, dt)`) × any source — technique dimension-driven horizon MERGE / join-shape cardinality proof: UNSUPPORTED — `join_shape::JoinContext` can only declare a SINGLE column as unique; a genuine composite-key equi-join (proven one-to-one in ground truth) is conservatively misclassified `OneToMany`, refusing a horizon MERGE it could safely take. Over-conservative, not unsound; `fan_out`/`dimension_horizon_merge` have no production call sites today, so no live path is affected (see ledger cell G-10).
self-referential batched model (direct-join form, no subquery wrap) × any source — technique outer time-filter clamp injection: UNSUPPORTED — `inject_time_filter` (`crates/smelt-runtime/src/transformer.rs`) injects the output clamp as a bare, unqualified column reference; a self-referential model's own driving source and self-reference both expose the partition column under its own name, so DuckDB rejects the compiled SQL with `Binder Error: Ambiguous reference to column name`. `docs/specs/batched_models.md`'s own documented pattern and `window_independence`'s own unit tests use this direct-join form. Fix requires a design choice (qualify to the resolved driving-fact alias vs. always wrap the query in an outer subquery) — BLOCKED for human review (see ledger cell G-11).
keyed additive fold (`cumulative_aggregate`) × any source — technique reprocessed-window refusal / never-fold-twice ledger check: UNSUPPORTED — the live `merge_into` run path has no watermark/ledger consultation (`cumulative.rs` step 2 is an admitted placeholder), so re-running an already-merged window silently double-folds the same delta into the keyed state (empirically pinned: 3 → 5). `keyed_models.md` §Reprocessing specs a `KeyedReprocessedWindow` refusal that the run path does not implement; the fix is the generalized reconciliation ledger's fold-refusal operation (see ledger cell G-12).
cross-partition scope (`DISTINCT`/`HAVING`/unaligned `OVER`/`LIMIT`) inside a CTE body, derived table, or set-operation arm × any source — technique batched admission + partition rewrite: UNSUPPORTED — the scope's key set does not pin the partition, so a row landing in one partition changes output rows of other, already-written partitions the rewrite never revisits (witness: SC-7's late-append schedule, maintained 1 row vs oracle 2). Batched admission now refuses these scopes wherever they nest (previously fail-open for CTE bodies; see ledger cell SC-7).
declared functional dependency (`key → determines`) whose `determines` column crosses a bare `UNION ALL` / set-operation body × any source — technique once-write / FD-widened enrichment: UNSUPPORTED — an FD holding in each arm does not hold in the union (the same key may appear in both arms with different determined values, e.g. `(c1,'EU')` vs `(c1,'US')`), so the declaration cannot be honoured. The walk-derived `PropertyVector` records the set-operation FD barrier and `functional_dependency_verdict_over_vector` refuses the declaration unless a literal discriminator in the key makes the branches provably disjoint. Analyzer now sound (was widening `None → Constant`); no once-write consumer wired, so no live path affected (see ledger cell SC-6).
