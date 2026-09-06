pub mod affected_keys;
pub mod bounded_domain;
pub mod decomposed_state;
pub mod definition_change;
pub mod diff;
pub mod diff_render;
pub mod diff_stories;
pub mod discriminants;
pub(crate) mod expr_util;
pub mod faithful_fold;
pub mod fingerprint;
pub mod footprint;
pub mod functional_dependency;
pub mod horizon_ceiling;
pub mod input_delta;
pub mod join_shape;
pub mod key_derived;
pub mod locality_projection;
pub mod model_diff;
pub mod monotonicity;
pub mod not_null;
pub mod output_delta;
mod partition_alignment;
pub mod partition_axis;
pub mod presentation;
pub mod profile;
mod select_analysis;
pub mod skeleton_closure;
pub mod source_bounds;
pub mod succession;
pub mod temporal;
pub mod walk;
pub mod window_independence;

pub use walk::{
    enumerate_scopes, model_property_vector, walk, ColumnDeterminism, ColumnDiscriminant,
    ColumnLineage, CteDef, DerivedFd, Determinism, Grain, InputItem, KeySet, LeafColumn, LeafInput,
    NodeCx, OpNode, PathSeg, PropertyTransfer, PropertyVector, QueryNode, QueryTree,
    RelationSource, Scope, ScopeEnum, ScopeEnumeration, ScopeKind, SelectNode, SetOpKind,
    SetOpNode, Transfer, UnsupportedConstruct,
};
// `is_constant_literal`/`constant_literal_tag` are `pub(crate)` (crate-internal
// leaf classifiers, not part of this crate's public surface) — accessed via
// `crate::analysis::walk::{..}` directly rather than re-exported here.

pub use partition_alignment::{
    resolve_scope_group_by, scope_distinct_alignment, scope_group_by_alignment,
    scope_over_alignment, window_has_bounded_range_interval_frame, window_over_alignment,
    PartitionAlignment,
};
pub use select_analysis::{
    analyze_select, classify_select_items, find_item_expr_by_alias_or_position,
    has_distinct_keyword, item_alias, item_expr, select_stmt_items, SelectAnalysis, SelectItemKind,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyze_basic_select() {
        let sql = "SELECT country, COUNT(DISTINCT user_id) as unique_users FROM events GROUP BY 1";
        let analysis = analyze_select(sql).unwrap();
        assert_eq!(analysis.items.len(), 2);
        assert!(
            matches!(&analysis.items[0], SelectItemKind::GroupByKey { text, .. } if text == "country")
        );
        assert!(
            matches!(&analysis.items[1], SelectItemKind::CountDistinct { argument, alias, .. } if argument == "user_id" && alias == "unique_users")
        );
        assert_eq!(analysis.group_by_exprs, vec!["country"]);
    }

    #[test]
    fn analyze_select_retains_expr_for_group_key() {
        let sql = "SELECT DATE_TRUNC('day', ts) AS d, COUNT(*) as cnt FROM events GROUP BY 1";
        let analysis = analyze_select(sql).unwrap();
        let SelectItemKind::GroupByKey { text, expr, .. } = &analysis.items[0] else {
            panic!("expected GroupByKey item");
        };
        assert_eq!(expr.text().trim(), text.as_str());
    }

    #[test]
    fn test_analyze_multiple_count_distinct() {
        let sql = r#"
            SELECT
                date_trunc('day', event_time) as event_date,
                country,
                COUNT(DISTINCT user_id) as unique_users,
                COUNT(DISTINCT session_id) as unique_sessions
            FROM events
            GROUP BY 1, 2
        "#;
        let analysis = analyze_select(sql).unwrap();

        let count_distincts: Vec<_> = analysis
            .items
            .iter()
            .filter(|i| matches!(i, SelectItemKind::CountDistinct { .. }))
            .collect();
        assert_eq!(count_distincts.len(), 2);

        let group_keys: Vec<_> = analysis
            .items
            .iter()
            .filter(|i| matches!(i, SelectItemKind::GroupByKey { .. }))
            .collect();
        assert_eq!(group_keys.len(), 2);
    }

    #[test]
    fn test_cube_split_annotation_detected() {
        let sql = "SELECT a, COUNT(DISTINCT b) as cb FROM t GROUP BY 1 -- smelt:cube_split";
        let analysis = analyze_select(sql).unwrap();
        assert!(analysis.has_cube_split_annotation);
    }

    #[test]
    fn test_no_cube_split_annotation() {
        let sql = "SELECT a, COUNT(DISTINCT b) as cb FROM t GROUP BY 1";
        let analysis = analyze_select(sql).unwrap();
        assert!(!analysis.has_cube_split_annotation);
    }

    #[test]
    fn test_analyze_with_where_clause() {
        let sql = "SELECT a, COUNT(*) as cnt FROM t WHERE active = true GROUP BY 1";
        let analysis = analyze_select(sql).unwrap();
        assert!(analysis.where_text.is_some());
        let where_text = analysis.where_text.unwrap();
        assert!(where_text.contains("active"));
    }

    #[test]
    fn test_other_aggregates() {
        let sql = "SELECT country, COUNT(*) as cnt, SUM(revenue) as total FROM t GROUP BY 1";
        let analysis = analyze_select(sql).unwrap();

        let others: Vec<_> = analysis
            .items
            .iter()
            .filter(|i| matches!(i, SelectItemKind::OtherAggregate { .. }))
            .collect();
        assert_eq!(others.len(), 2);
    }

    #[test]
    fn test_ordinal_resolution() {
        let sql = "SELECT country, city, COUNT(DISTINCT user_id) as users FROM t GROUP BY 1, 2";
        let analysis = analyze_select(sql).unwrap();
        assert_eq!(analysis.group_by_exprs.len(), 2);
        assert_eq!(analysis.group_by_exprs[0], "country");
        assert_eq!(analysis.group_by_exprs[1], "city");
    }

    #[test]
    fn test_from_text_preserved() {
        let sql = "SELECT a FROM smelt.models.events e GROUP BY 1";
        let analysis = analyze_select(sql).unwrap();
        assert!(analysis.from_text.contains("smelt.models.events"));
    }

    #[test]
    fn test_frontmatter_stripped() {
        let sql = "---\nmaterialized: table\n---\nSELECT a FROM t GROUP BY 1";
        let analysis = analyze_select(sql).unwrap();
        assert_eq!(analysis.items.len(), 1);
    }

    #[test]
    fn test_group_by_not_in_comment() {
        // GROUP BY in a line comment must not be confused with the actual GROUP BY.
        // Regression: models with "GROUP BY" in a comment caused extraction to grab
        // the wrong position (comment text instead of the real clause).
        let sql = r#"
            -- session_start_date appears in both the SELECT list and the GROUP BY.
            SELECT
                s.session_id,
                s.session_start_date,
                'u:' || CAST(arg_max(e.user_id, e.event_ts) AS VARCHAR) AS fwd
            FROM sessions s
            GROUP BY s.session_id, s.session_start_date
        "#;
        let analysis = analyze_select(sql).unwrap();
        // Must extract from the real GROUP BY, not the comment.
        assert!(
            analysis
                .group_by_exprs
                .contains(&"s.session_start_date".to_string()),
            "expected s.session_start_date in group_by_exprs; got: {:?}",
            analysis.group_by_exprs
        );
    }

    fn parse_select(sql: &str) -> smelt_parser::SelectStmt {
        let parse = smelt_parser::parse(sql);
        let file = smelt_parser::File::cast(parse.syntax()).expect("file");
        file.select_stmt().expect("select stmt")
    }

    #[test]
    fn test_scope_group_by_alignment_aligned() {
        let select = parse_select(
            "SELECT event_date, user_id, COUNT(*) as cnt FROM events \
             GROUP BY event_date, user_id HAVING COUNT(*) > 1",
        );
        assert_eq!(
            scope_group_by_alignment(&select, "event_date"),
            PartitionAlignment::Aligned
        );
    }

    #[test]
    fn test_scope_group_by_alignment_not_aligned_fails_closed() {
        // GROUP BY omits the partition_column entirely.
        let select = parse_select(
            "SELECT event_date, user_id, COUNT(*) as cnt FROM events \
             GROUP BY user_id HAVING COUNT(*) > 1",
        );
        assert!(!scope_group_by_alignment(&select, "event_date").is_aligned());
    }

    #[test]
    fn test_scope_group_by_alignment_no_group_by_fails_closed() {
        let select = parse_select("SELECT a, b FROM t");
        assert!(!scope_group_by_alignment(&select, "a").is_aligned());
    }

    #[test]
    fn test_grouping_sets_grain_verdict_mirrors_cube_verdict() {
        // Neither `CUBE(...)` nor `GROUPING SETS (...)` has dedicated
        // smelt-side grammar for grain/FD purposes — both flow through
        // `resolve_scope_group_by` as one opaque grouping-key expression
        // whose text is the whole construct (e.g. "CUBE(event_date, user_id)"
        // / "GROUPING SETS ((event_date), (user_id))"). That text never
        // matches a plain projected column name, so both are conservatively
        // judged `NotAligned` — never a phantom `Aligned` claim that
        // `event_date` (or any other column) is a genuine grouping key of
        // this scope. This is the "same verdict class" the GROUPING SETS
        // implementation is required to mirror from the CUBE/ROLLUP
        // precedent (there being no richer precedent to match, since neither
        // gets special-cased grain treatment today).
        let cube_select = parse_select(
            "SELECT event_date, user_id, COUNT(*) as cnt FROM events \
             GROUP BY CUBE(event_date, user_id) HAVING COUNT(*) > 1",
        );
        let cube_verdict = scope_group_by_alignment(&cube_select, "event_date");
        assert!(
            !cube_verdict.is_aligned(),
            "CUBE grouping key text never matches a plain column name: {cube_verdict:?}"
        );

        let grouping_sets_select = parse_select(
            "SELECT event_date, user_id, COUNT(*) as cnt FROM events \
             GROUP BY GROUPING SETS ((event_date), (user_id)) HAVING COUNT(*) > 1",
        );
        let grouping_sets_verdict = scope_group_by_alignment(&grouping_sets_select, "event_date");
        assert!(
            !grouping_sets_verdict.is_aligned(),
            "GROUPING SETS must mirror CUBE's conservative verdict: {grouping_sets_verdict:?}"
        );

        // Both land in the same verdict variant (`NotAligned`), not merely
        // both "not Aligned" by coincidence of different enum shapes.
        assert!(matches!(
            cube_verdict,
            PartitionAlignment::NotAligned { .. }
        ));
        assert!(matches!(
            grouping_sets_verdict,
            PartitionAlignment::NotAligned { .. }
        ));

        // Sanity: resolve_scope_group_by sees exactly one opaque key for
        // each — the whole construct's own text — confirming there is no
        // phantom expansion into ["event_date", "user_id"].
        let items = select_stmt_items(&cube_select).unwrap_or_default();
        let cube_keys = resolve_scope_group_by(&cube_select, &items);
        assert_eq!(cube_keys.len(), 1);
        assert!(cube_keys[0].to_uppercase().starts_with("CUBE"));

        let gs_items = select_stmt_items(&grouping_sets_select).unwrap_or_default();
        let gs_keys = resolve_scope_group_by(&grouping_sets_select, &gs_items);
        assert_eq!(gs_keys.len(), 1);
        assert!(gs_keys[0].to_uppercase().starts_with("GROUPING SETS"));
    }

    #[test]
    fn test_scope_distinct_alignment_aligned_when_projected() {
        let select = parse_select("SELECT DISTINCT event_date, user_id FROM events");
        assert!(scope_distinct_alignment(&select, "event_date").is_aligned());
    }

    #[test]
    fn test_scope_distinct_alignment_not_aligned_when_not_projected() {
        let select = parse_select("SELECT DISTINCT user_id FROM events");
        assert!(!scope_distinct_alignment(&select, "event_date").is_aligned());
    }

    #[test]
    fn test_scope_over_alignment_aligned_when_partition_by_superset() {
        let select = parse_select(
            "SELECT event_date, user_id, \
             SUM(amount) OVER (PARTITION BY event_date, user_id ORDER BY user_id) AS running \
             FROM events",
        );
        assert_eq!(
            scope_over_alignment(&select, "event_date"),
            PartitionAlignment::Aligned
        );
    }

    #[test]
    fn test_scope_over_alignment_not_aligned_when_partition_by_omits_column() {
        let select = parse_select(
            "SELECT event_date, user_id, \
             SUM(amount) OVER (PARTITION BY user_id ORDER BY user_id) AS running \
             FROM events",
        );
        assert!(!scope_over_alignment(&select, "event_date").is_aligned());
    }

    #[test]
    fn test_scope_over_alignment_is_per_scope_not_outer() {
        // The outer query has no window at all; the FROM subquery's own
        // window is aligned. Reading the outer scope must not see the
        // subquery's alignment, and reading the subquery's own scope must
        // see it correctly regardless of the outer query's shape.
        let outer = parse_select(
            "SELECT * FROM (\
                 SELECT event_date, user_id, \
                 SUM(amount) OVER (PARTITION BY event_date ORDER BY user_id) AS running \
                 FROM events\
             ) t",
        );
        assert!(
            !scope_over_alignment(&outer, "event_date").is_aligned(),
            "outer scope has no window OVER of its own"
        );

        let inner = outer
            .from_clause()
            .expect("from clause")
            .table_refs()
            .next()
            .expect("table ref")
            .subquery()
            .expect("subquery")
            .select_stmt()
            .expect("inner select");
        assert!(
            scope_over_alignment(&inner, "event_date").is_aligned(),
            "inner scope's own window is partition-aligned"
        );
    }

    #[test]
    fn test_scope_over_alignment_fails_closed_when_no_partition_by() {
        // A window with no PARTITION BY at all must never be optimistically
        // treated as aligned.
        let select = parse_select(
            "SELECT event_date, ROW_NUMBER() OVER (ORDER BY event_date) AS rn FROM events",
        );
        assert!(!scope_over_alignment(&select, "event_date").is_aligned());
    }

    #[test]
    fn test_scope_over_alignment_no_window_fails_closed() {
        let select = parse_select("SELECT event_date, user_id FROM events");
        assert!(!scope_over_alignment(&select, "event_date").is_aligned());
    }

    /// The alignment verdict is computed **per-scope**: a UNION's second
    /// branch has its own `GROUP BY` (omitting the partition_column), which
    /// must be judged on its own terms — not the first branch's (aligned)
    /// `GROUP BY`.
    #[test]
    fn test_alignment_is_per_scope_not_outer() {
        let outer = parse_select(
            "SELECT event_date, user_id, COUNT(*) as cnt FROM events_a \
             GROUP BY event_date, user_id \
             UNION ALL \
             SELECT event_date, user_id, COUNT(*) as cnt FROM events_b \
             GROUP BY user_id",
        );
        assert!(scope_group_by_alignment(&outer, "event_date").is_aligned());

        let branch2 = outer.union_select().expect("second UNION branch");
        assert!(
            !scope_group_by_alignment(&branch2, "event_date").is_aligned(),
            "branch 2's own GROUP BY (user_id only) must not inherit branch 1's alignment"
        );
    }

    /// A GROUP BY column whose name is *prefixed* by an end-keyword
    /// (`order_id` contains `ORDER`, `having_flag` contains no keyword but is
    /// listed defensively, etc.) must survive as the sole derived key — the
    /// end-keyword scan must not treat `_` as a non-identifier boundary char.
    #[test]
    fn group_by_column_prefixed_by_an_end_keyword_survives() {
        let columns = [
            "order_id",
            "having_flag",
            "union_all",
            "limit_count",
            "except_code",
            "intersect_key",
            "fetch_size",
        ];
        for col in columns {
            let sql = format!("SELECT {col}, COUNT(*) as cnt FROM t GROUP BY {col}");
            let analysis =
                analyze_select(&sql).unwrap_or_else(|| panic!("failed to analyze {sql}"));
            assert_eq!(
                analysis.group_by_exprs,
                vec![col.to_string()],
                "GROUP BY column `{col}` was truncated by an end-keyword collision"
            );
        }
    }

    #[test]
    fn real_order_by_after_group_by_still_terminates_the_clause() {
        let sql = "SELECT a, COUNT(*) as cnt FROM t GROUP BY a ORDER BY a";
        let analysis = analyze_select(sql).unwrap();
        assert_eq!(analysis.group_by_exprs, vec!["a"]);

        let sql_lower = "SELECT a, COUNT(*) as cnt FROM t GROUP BY a order by a";
        let analysis = analyze_select(sql_lower).unwrap();
        assert_eq!(analysis.group_by_exprs, vec!["a"]);

        let sql_having = "SELECT a, COUNT(*) as cnt FROM t GROUP BY a HAVING COUNT(*) > 1";
        let analysis = analyze_select(sql_having).unwrap();
        assert_eq!(analysis.group_by_exprs, vec!["a"]);
    }

    #[test]
    fn quoted_or_qualified_end_keyword_is_not_a_clause_terminator() {
        let sql = r#"SELECT t."order", COUNT(*) as cnt FROM t GROUP BY t."order""#;
        let analysis = analyze_select(sql).unwrap();
        assert_eq!(analysis.group_by_exprs, vec![r#"t."order""#]);

        let sql_quoted = r#"SELECT "order", COUNT(*) as cnt FROM t GROUP BY "order""#;
        let analysis = analyze_select(sql_quoted).unwrap();
        assert_eq!(analysis.group_by_exprs, vec![r#""order""#]);
    }
}
