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
