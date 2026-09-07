/// The SQL call-site context an emission verdict is stated for.
///
/// A built-in's support on a backend routinely differs between the positions
/// it can appear in — GoogleSQL refuses `PERCENTILE_CONT` under a `GROUP BY`
/// but accepts it with an `OVER` clause, while `MAX_BY` is the exact reverse
/// — so a verdict is looked up by `(dialect, position)`, never by dialect
/// alone. `Any` is a lookup wildcard for an entry whose verdict does not vary
/// by position; it is never returned by a classifier that decides a call's
/// actual position from its source CST — such a classifier always resolves
/// to one of the other four variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Position {
    /// Lookup wildcard, matching any call position. Never returned by a
    /// position classifier — only ever used as a stated verdict key.
    Any,
    /// A row-wise expression: no `OVER` clause, and not itself an aggregate
    /// call. A scalar call under a `GROUP BY` (e.g. applied to a grouping
    /// key or in a `WHERE` clause) is still `Scalar` — the enclosing
    /// statement's `GROUP BY` does not change a call's own position.
    Scalar,
    /// The call is itself an aggregate call, with no `OVER` clause.
    Aggregate,
    /// An `OVER` clause whose window covers the call's whole partition —
    /// after resolving any named-window reference, no window `ORDER BY` and
    /// no frame clause, or an explicit `BETWEEN UNBOUNDED PRECEDING AND
    /// UNBOUNDED FOLLOWING` frame with no `EXCLUDE` clause.
    WholePartitionWindow,
    /// An `OVER` clause whose window is narrower than its whole partition —
    /// includes the common `ORDER BY` with no explicit frame (whose SQL
    /// default frame is `RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT
    /// ROW`), any frame carrying `EXCLUDE`, and an unresolvable named-window
    /// reference (refusing is the safe direction: it costs a diagnostic,
    /// where guessing costs a wrong number).
    Window,
}

/// A structural rewrite the printer implements. Enumerable by construction, so
/// the set of rewrites is knowable without reading the printer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RewriteId {
    /// `MEDIAN(x)` → `PERCENTILE_CONT(x, 0.5)` in window position, an
    /// `ARRAY_AGG`-indexing `CASE` in aggregate position. Position-dependent;
    /// the registry says *that* it needs rewriting, the printer says *how*.
    ///
    /// Not a template: the output shape itself differs by call position (a
    /// single substitution in window position, a multi-statement `CASE` over
    /// an `ARRAY_AGG` in aggregate position) — a `{n}` placeholder names an
    /// argument, not a choice of output shape.
    BigQueryMedian,
    /// `PERCENTILE_CONT(f) WITHIN GROUP (ORDER BY x)` → `PERCENTILE_CONT(x,
    /// f)` at a whole-partition window position — GoogleSQL's two-argument
    /// analytic spelling, since `WITHIN GROUP` under an `OVER` clause is a
    /// syntax error there (measured live 2026-08-27). The window itself is
    /// left as-is; only the call's own spelling changes. A `DESC` sort key
    /// inverts the fraction argument; a `NULLS FIRST`/`LAST` modifier the
    /// analytic form cannot express is refused upstream by
    /// `emission_check`, never reaching the printer
    /// (`restructure::within_group_sort_key` is the shared reader for both
    /// this rewrite and `RestructureId::AnalyticToCte`).
    ///
    /// Not a template: the sort key and its direction come from the call's
    /// own `WITHIN GROUP (ORDER BY …)` clause, a construct a positional `{n}`
    /// placeholder cannot address — the rewrite reads that clause with
    /// `within_group_sort_key` rather than substituting a positional argument.
    WithinGroupToAnalytic,
}

/// A statement-level restructure shape. Enumerable by construction, mirroring
/// [`RewriteId`] — the set of shapes is knowable without reading the planner.
///
/// Correctness oracle: `docs/specs/multi_backend.md` §"Statement-level
/// lowering".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RestructureId {
    /// An aggregate-only built-in reached with an `OVER` clause (GoogleSQL's
    /// `MAX_BY`/`MIN_BY`/`APPROX_COUNT_DISTINCT`; DuckDB's and Spark's
    /// ordered-set `PERCENTILE_CONT`/`PERCENTILE_DISC`). The source is bound
    /// once, grouped by the call's partition keys, and joined back —
    /// admissible only at `Position::WholePartitionWindow`.
    WindowToCte,
    /// An analytic-only built-in reached under `GROUP BY` (GoogleSQL's
    /// `PERCENTILE_CONT`/`PERCENTILE_DISC`, which require an `OVER` clause
    /// and reject `WITHIN GROUP` outright). The query's `FROM`/`WHERE` move
    /// into a CTE that adds the value as an analytic column over the
    /// grouping keys, read back through `ANY_VALUE`.
    AnalyticToCte,
}
