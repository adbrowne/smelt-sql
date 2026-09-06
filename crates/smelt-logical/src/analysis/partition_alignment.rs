use crate::analysis::{item_alias, item_expr, select_stmt_items, SelectItemKind};

/// The verdict for whether a SELECT scope's own `GROUP BY` / `DISTINCT` key
/// is a superset of the model's `partition_column` — the shared
/// partition-alignment signal (`incremental_shapes.md` §"Safety checks") that
/// licenses group-aligned `HAVING`/`DISTINCT` admission
/// (`rules::incremental`) and is available to other per-scope consumers
/// (UNION-branch / window admission) as the same reusable check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartitionAlignment {
    /// The scope's own key is a superset of `partition_column` — safe.
    Aligned,
    /// The scope's own key does not contain `partition_column`; `reason`
    /// names why (no `GROUP BY` in this scope, `partition_column` not
    /// projected here, or the key omits it).
    NotAligned { reason: String },
}

impl PartitionAlignment {
    pub fn is_aligned(&self) -> bool {
        matches!(self, PartitionAlignment::Aligned)
    }
}

/// Leaf classifier (`docs/specs/architecture.md` §"Property composition walk
/// rule"): the DuckDB `GROUP BY ALL` expansion — every non-aggregate select
/// item of this one already-classified scope becomes a grouping key, mirroring
/// DuckDB's own semantics. It reasons only over the caller's already-bounded
/// `items`, never over raw SQL text, so it is the single source of expansion
/// truth shared by both the AST path ([`resolve_scope_group_by`]) and the
/// text-scan path ([`analyze_select`]).
pub(crate) fn group_by_all_keys(items: &[SelectItemKind]) -> Vec<String> {
    items
        .iter()
        .filter(|item| matches!(item, SelectItemKind::GroupByKey { .. }))
        .map(|item| item_expr(item).text().trim().to_string())
        .collect()
}

/// Resolve `select`'s own `GROUP BY` expressions against **its own**
/// select-list `items` — ordinal references (`GROUP BY 1, 2`) resolve to
/// this scope's projections, not an outer query's. `GROUP BY ALL` expands to
/// the scope's non-aggregate select items (DuckDB semantics). Returns an empty
/// `Vec` when the scope has no `GROUP BY` clause (or a `GROUP BY ALL` over an
/// aggregates-only projection — single-group semantics).
pub fn resolve_scope_group_by(
    select: &smelt_parser::SelectStmt,
    items: &[SelectItemKind],
) -> Vec<String> {
    let Some(group_by) = select.group_by_clause() else {
        return Vec::new();
    };
    // DuckDB `GROUP BY ALL`: the clause carries no explicit key expressions —
    // it groups by every non-aggregate select item. Expand to those items'
    // expression text so the walk sees the real grouping keys (never a phantom
    // `ALL` key). An aggregates-only projection yields an empty key set, i.e.
    // single-group semantics, identical to a plain aggregate without GROUP BY.
    if group_by.is_all() {
        return group_by_all_keys(items);
    }
    group_by
        .expressions()
        .map(|expr| {
            let text = expr.text().trim().to_string();
            if let Ok(ordinal) = text.parse::<usize>() {
                if ordinal >= 1 {
                    if let Some(item) = items.get(ordinal - 1) {
                        return item_expr(item).text().trim().to_string();
                    }
                }
            }
            text
        })
        .collect()
}

/// Partition-alignment verdict for a `HAVING` clause living in `select`:
/// `Aligned` when this scope's own resolved `GROUP BY` keys are a superset
/// containing the projected `partition_col` expression — found among
/// `select`'s **own** select-list items, so a subquery/UNION-branch body is
/// judged by its own projections and its own `GROUP BY`, never the outer
/// query's (`incremental_shapes.md` §"Safety checks").
pub fn scope_group_by_alignment(
    select: &smelt_parser::SelectStmt,
    partition_col: &str,
) -> PartitionAlignment {
    let items = select_stmt_items(select).unwrap_or_default();
    let Some(partition_item) = items.iter().find(|item| item_alias(item) == partition_col) else {
        return PartitionAlignment::NotAligned {
            reason: format!("partition_column '{partition_col}' is not projected in this scope"),
        };
    };
    let partition_expr = item_expr(partition_item).text().trim().to_string();
    let group_by_keys = resolve_scope_group_by(select, &items);
    if group_by_keys.is_empty() {
        return PartitionAlignment::NotAligned {
            reason: "this scope has no GROUP BY".to_string(),
        };
    }
    if group_by_keys.iter().any(|k| k == &partition_expr) {
        PartitionAlignment::Aligned
    } else {
        PartitionAlignment::NotAligned {
            reason: format!(
                "GROUP BY ({}) does not include the partition_column '{partition_col}' expression '{partition_expr}'",
                group_by_keys.join(", ")
            ),
        }
    }
}

/// Partition-alignment verdict for a `SELECT DISTINCT` living in `select`:
/// the dedup key is the whole projected row, so it is `Aligned` whenever
/// `partition_col` is itself projected in this scope's own select list
/// (`timeseries.md`'s partition-column-projection rule already requires
/// this at the outer scope; checked independently here so an inner scope
/// that does *not* project `partition_col` is not erroneously admitted).
pub fn scope_distinct_alignment(
    select: &smelt_parser::SelectStmt,
    partition_col: &str,
) -> PartitionAlignment {
    let items = select_stmt_items(select).unwrap_or_default();
    if items.iter().any(|item| item_alias(item) == partition_col) {
        PartitionAlignment::Aligned
    } else {
        PartitionAlignment::NotAligned {
            reason: format!("partition_column '{partition_col}' is not projected in this scope"),
        }
    }
}

/// Partition-alignment verdict for the window `OVER` scope(s) living in
/// `select`'s own select list: `Aligned` when **every** window found there
/// has a `PARTITION BY` whose keys are a superset containing `partition_col`
/// (AST-based, replacing the substring `OVER`/`PARTITION BY` scan). A scope
/// with no window at all, or any window missing `PARTITION BY` or omitting
/// `partition_col`, fails closed to `NotAligned{reason}` — never optimistic.
/// Judged against **this** scope's own select list only (a FROM subquery's
/// window is read by resolving to that subquery's own `SelectStmt` first).
pub fn scope_over_alignment(
    select: &smelt_parser::SelectStmt,
    partition_col: &str,
) -> PartitionAlignment {
    let items = select_stmt_items(select).unwrap_or_default();
    let windows: Vec<smelt_parser::WindowSpec> = items
        .iter()
        .filter_map(|item| item_expr(item).window_spec())
        .collect();
    if windows.is_empty() {
        return PartitionAlignment::NotAligned {
            reason: "this scope has no window OVER clause".to_string(),
        };
    }
    for window in &windows {
        if let not_aligned @ PartitionAlignment::NotAligned { .. } =
            window_over_alignment(window, partition_col)
        {
            return not_aligned;
        }
    }
    PartitionAlignment::Aligned
}

/// Partition-alignment verdict for a single window `OVER` clause: `Aligned`
/// when its `PARTITION BY` keys are a superset containing `partition_col`.
/// A window with no `PARTITION BY` (including a named-window reference,
/// `OVER w`) fails closed to `NotAligned` — never optimistic. This is the
/// per-window leaf classifier the composition walk invokes for every window
/// scope it enumerates (`model_properties.md` §"The composition walk").
pub fn window_over_alignment(
    window: &smelt_parser::WindowSpec,
    partition_col: &str,
) -> PartitionAlignment {
    let Some(partition_by) = window.partition_by() else {
        return PartitionAlignment::NotAligned {
            reason: "a window OVER clause in this scope has no PARTITION BY".to_string(),
        };
    };
    let keys: Vec<String> = partition_by
        .expressions()
        .map(|e| e.text().trim().to_string())
        .collect();
    if !keys.iter().any(|k| k == partition_col) {
        return PartitionAlignment::NotAligned {
            reason: format!(
                "window OVER (PARTITION BY {}) does not include the partition_column '{partition_col}'",
                keys.join(", ")
            ),
        };
    }
    PartitionAlignment::Aligned
}

/// True when a window's frame is a bounded `RANGE BETWEEN INTERVAL '…'
/// PRECEDING [AND …]` (Form A) with no `UNBOUNDED` bound. Such a frame has a
/// finite reach the source-bound deriver picks up and the planner widens the
/// source read to cover, so the window is partition-local up to that bound —
/// admissible even when its `PARTITION BY` omits the partition column
/// (alignment is the zero-lookback license; frame-reach is the
/// bounded-lookback one). An `UNBOUNDED` bound reads across all history and
/// is deliberately excluded. Leaf classifier over the frame clause's AST.
pub fn window_has_bounded_range_interval_frame(window: &smelt_parser::WindowSpec) -> bool {
    let Some(frame) = window.frame() else {
        return false;
    };
    if frame.unit() != Some(smelt_parser::FrameUnit::Range) {
        return false;
    }
    let bounds = frame.bounds();
    // `BETWEEN <lo> AND <hi>` yields two bounds; the single-bound spelling
    // (`RANGE INTERVAL '…' PRECEDING`) is not the derivable Form A shape.
    if bounds.len() != 2 {
        return false;
    }
    let has_interval = bounds
        .iter()
        .any(|b| b.text().to_uppercase().contains("INTERVAL"));
    let has_unbounded = bounds
        .iter()
        .any(|b| b.text().to_uppercase().contains("UNBOUNDED"));
    has_interval && !has_unbounded
}
