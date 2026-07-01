# Monotonicity-primitive research backing material

Raw research notes that back **Parts 6 and 7** of
[`../20260701-expanding-incremental-eligibility.md`](../20260701-expanding-incremental-eligibility.md).
Each file is the full, un-condensed output of one research pass; the parent
document distils them. Kept here so the citations and per-system detail remain
recoverable without re-running the research.

| File | Feeds | Contents |
|---|---|---|
| [`monotonicity-primitive-deepdive.md`](monotonicity-primitive-deepdive.md) | Part 6 | Codebase-grounded design of the monotonicity primitive: precise definition, static whitelist, declared-guarantee fallback, proposed `smelt-logical` module shape, edge cases. |
| [`research-ivm-and-monotonicity-theory.md`](research-ivm-and-monotonicity-theory.md) | §7.3 | Academic theory: incremental view maintenance, the CALM theorem, DBSP, watermarks, and the undecidability limits — annotated bibliography. |
| [`research-industry-implementations.md`](research-industry-implementations.md) | §7.1, §7.2, §7.5 | How production engines (Databricks Enzyme, Snowflake Dynamic Tables, BigQuery MVs, Spark/Flink, Materialize, dbt, SQLMesh, ClickHouse, Feldera, cube.dev) decide incremental-refresh eligibility, with a full comparison table. |
| [`research-pushdown-and-monotone-expressions.md`](research-pushdown-and-monotone-expressions.md) | §7.4 | Predicate-pushdown soundness laws and the closest production analogs of the primitive (ClickHouse `getMonotonicityForRange`, Iceberg partition transforms, Delta generated columns), plus a synthesized monotone-builtin whitelist. |

The consolidated citation list lives in the parent document's **References**
section; these files carry the fuller annotations and verification notes.
