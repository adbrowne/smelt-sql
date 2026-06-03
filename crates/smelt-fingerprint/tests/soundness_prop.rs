//! Soundness property test — the load-bearing gate of Stage 0.
//!
//! Invariant: for every (base query, transformation),
//!
//! ```text
//!   fingerprint(base) == fingerprint(transform(base))  ⇒  relations_equal(run(base), run(transform(base)))
//! ```
//!
//! Only this direction is asserted. Incompleteness — the fingerprint failing to
//! recognise a genuine equivalence — is allowed and asserts nothing. A failure
//! here means a false "equivalent": the fingerprint said two queries were the
//! same when DuckDB proves they are not. That is the data-corruption bug the
//! whole design must never commit, so this test must exist before any reuse is
//! wired to execution.
//!
//! The generator emits BOTH output-preserving transforms (which exercise the
//! "fp equal ⇒ rows equal" path) and semantics-changing transforms (which
//! exercise that a real change does not slip through with an equal fingerprint).
//!
//! Honour `PROPTEST_CASES` for deeper local runs.

mod oracle;
use oracle::{relations_equal, DuckDbRelationOracle};
use proptest::prelude::*;
use smelt_fingerprint::output_fingerprint_from_sql;

const COLS: [&str; 3] = ["a", "b", "c"];
/// Three rows over (a INT, b INT, c DOUBLE). Distinct values so filters,
/// projections and DISTINCT all have material effect.
const SEED_BODY: &str =
    "SELECT 1 AS a, 2 AS b, 1.5 AS c UNION ALL SELECT 4, 0, 2.5 UNION ALL SELECT 7, 3, 9.0";

#[derive(Debug, Clone)]
struct ProjEntry {
    name: String,
    expr: String,
}

#[derive(Debug, Clone)]
enum Transform {
    /// Output-preserving: rotate the projection order.
    ReorderProj,
    /// Output-preserving: rename the CTE binding.
    RenameCte,
    /// Output-preserving: add a comment.
    AddComment,
    /// Output-preserving: lowercase keywords.
    LowercaseKeywords,
    /// Semantics-changing: `expr` → `(expr) + 1` on projection `idx`.
    BumpExpr(usize),
    /// Semantics-changing: drop the first projection column (if >1).
    DropFirst,
    /// Semantics-changing: add/replace a filter `col > k`.
    SetFilter(usize, i64),
    /// Output-preserving: rewrite the single-use CTE as a derived table.
    ToDerivedTable,
}

#[derive(Debug, Clone)]
struct Rendered {
    cte: String,
    proj: Vec<ProjEntry>,
    filter: Option<(String, i64)>,
    lower: bool,
    comment: bool,
    /// Render the source as a derived table `FROM (body) AS cte` rather than a
    /// `WITH cte AS (body) … FROM cte`.
    derived: bool,
}

impl Rendered {
    fn to_sql(&self) -> String {
        let kw = |s: &str| {
            if self.lower {
                s.to_lowercase()
            } else {
                s.to_string()
            }
        };
        let mut cols: Vec<String> = self
            .proj
            .iter()
            .map(|p| format!("{} {} {}", p.expr, kw("AS"), p.name))
            .collect();
        if self.comment {
            if let Some(last) = cols.last_mut() {
                last.push_str(" /* note */");
            }
        }
        let where_s = match &self.filter {
            Some((col, k)) => format!(" {} {} > {}", kw("WHERE"), col, k),
            None => String::new(),
        };
        if self.derived {
            format!(
                "{select} {cols} {from} ({body}) {as_} {cte}{where_s}",
                select = kw("SELECT"),
                cols = cols.join(", "),
                from = kw("FROM"),
                body = SEED_BODY,
                as_ = kw("AS"),
                cte = self.cte,
            )
        } else {
            format!(
                "{with} {cte} {as_} ({body}) {select} {cols} {from} {cte}{where_s}",
                with = kw("WITH"),
                cte = self.cte,
                as_ = kw("AS"),
                body = SEED_BODY,
                select = kw("SELECT"),
                cols = cols.join(", "),
                from = kw("FROM"),
            )
        }
    }
}

fn apply(base: &Rendered, t: &Transform) -> Rendered {
    let mut r = base.clone();
    match t {
        Transform::ReorderProj => {
            if r.proj.len() > 1 {
                r.proj.rotate_left(1);
            }
        }
        Transform::RenameCte => r.cte = format!("{}_renamed", r.cte),
        Transform::AddComment => r.comment = true,
        Transform::LowercaseKeywords => r.lower = true,
        Transform::BumpExpr(i) => {
            let idx = i % r.proj.len();
            r.proj[idx].expr = format!("({}) + 1", r.proj[idx].expr);
        }
        Transform::DropFirst => {
            if r.proj.len() > 1 {
                r.proj.remove(0);
            }
        }
        Transform::SetFilter(i, k) => {
            r.filter = Some((COLS[i % COLS.len()].to_string(), *k));
        }
        Transform::ToDerivedTable => r.derived = true,
    }
    r
}

fn proj_strategy() -> impl Strategy<Value = Vec<usize>> {
    // 1..=3 column indices, deduplicated preserving order (distinct columns).
    prop::collection::vec(0usize..3, 1..=3).prop_map(|v| {
        let mut seen = Vec::new();
        for i in v {
            if !seen.contains(&i) {
                seen.push(i);
            }
        }
        seen
    })
}

fn transform_strategy() -> impl Strategy<Value = Transform> {
    prop_oneof![
        Just(Transform::ReorderProj),
        Just(Transform::RenameCte),
        Just(Transform::AddComment),
        Just(Transform::LowercaseKeywords),
        (0usize..3).prop_map(Transform::BumpExpr),
        Just(Transform::DropFirst),
        (0usize..3, 0i64..10).prop_map(|(i, k)| Transform::SetFilter(i, k)),
        Just(Transform::ToDerivedTable),
    ]
}

// ===========================================================================
// Join soundness — the same invariant over two-table (derived-table) joins.
//
// This exercises the canonicaliser's FROM/join path, which the single-table
// generator above never reaches. The base is a derived-table-on-the-left join
// (the shape where an inlining bug previously dropped the JOIN entirely); the
// transforms mix output-preserving rewrites with join-semantics changes, and
// DuckDB is the judge of any equal-fingerprint pair.
// ===========================================================================

/// Two-table body. `a` values {1,4,5} and `b` values {2,0,5} give join columns
/// with varied match cardinalities (a=a: 3, a=b: 1, b=a: 1), so INNER vs LEFT
/// and on-column changes all produce observable row differences.
const JBODY: &str = "SELECT 1 AS a, 2 AS b UNION ALL SELECT 4, 0 UNION ALL SELECT 5, 5";
const JCOLS: [&str; 2] = ["a", "b"];

#[derive(Debug, Clone, Copy, PartialEq)]
enum Side {
    L,
    R,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum JoinKind {
    Inner,
    Left,
}

#[derive(Debug, Clone)]
struct JoinQuery {
    kind: JoinKind,
    on_left: usize,
    on_right: usize,
    extra: Option<(usize, i64)>, // AND l.<col> > k
    proj: Vec<(Side, usize)>,    // unique (side, col); non-empty
    lower: bool,
    comment: bool,
    swap: bool, // render right-table <join> left-table (textual order)
    l_alias: String,
    r_alias: String,
}

#[derive(Debug, Clone)]
enum JoinTransform {
    // Output-preserving
    ReorderProj,
    Lowercase,
    AddComment,
    RenameAliases,
    // Semantics-changing
    FlipKind,
    ChangeOnRight,
    AddExtraPred,
    SwapBranches,
}

impl JoinQuery {
    fn alias(&self, s: Side) -> &str {
        match s {
            Side::L => &self.l_alias,
            Side::R => &self.r_alias,
        }
    }
    fn out_name(s: Side, col: usize) -> String {
        // Stable output name keyed on side+column (not the alias), so an alias
        // rename does not change the relation's column names.
        format!("{}{}", if s == Side::L { "l" } else { "r" }, JCOLS[col])
    }

    fn to_sql(&self) -> String {
        let kw = |s: &str| {
            if self.lower {
                s.to_lowercase()
            } else {
                s.to_string()
            }
        };
        let mut cols: Vec<String> = self
            .proj
            .iter()
            .map(|&(s, c)| {
                format!(
                    "{}.{} {} {}",
                    self.alias(s),
                    JCOLS[c],
                    kw("AS"),
                    Self::out_name(s, c)
                )
            })
            .collect();
        if self.comment {
            if let Some(last) = cols.last_mut() {
                last.push_str(" /* note */");
            }
        }
        let join_kw = match self.kind {
            JoinKind::Inner => kw("INNER JOIN"),
            JoinKind::Left => kw("LEFT JOIN"),
        };
        // Textual table order may swap; ON always references l_alias/r_alias.
        let (a1, a2) = if self.swap {
            (&self.r_alias, &self.l_alias)
        } else {
            (&self.l_alias, &self.r_alias)
        };
        let extra = match self.extra {
            Some((c, k)) => format!(" {} {}.{} > {}", kw("AND"), self.l_alias, JCOLS[c], k),
            None => String::new(),
        };
        format!(
            "{select} {cols} {from} ({JBODY}) {as_} {a1} {join_kw} ({JBODY}) {as_} {a2} {on} {l}.{lc} = {r}.{rc}{extra}",
            select = kw("SELECT"),
            cols = cols.join(", "),
            from = kw("FROM"),
            as_ = kw("AS"),
            on = kw("ON"),
            l = self.l_alias,
            lc = JCOLS[self.on_left],
            r = self.r_alias,
            rc = JCOLS[self.on_right],
        )
    }
}

fn apply_join(base: &JoinQuery, t: &JoinTransform) -> JoinQuery {
    let mut q = base.clone();
    match t {
        JoinTransform::ReorderProj => {
            if q.proj.len() > 1 {
                q.proj.rotate_left(1);
            }
        }
        JoinTransform::Lowercase => q.lower = true,
        JoinTransform::AddComment => q.comment = true,
        JoinTransform::RenameAliases => {
            q.l_alias = format!("{}_x", q.l_alias);
            q.r_alias = format!("{}_y", q.r_alias);
        }
        JoinTransform::FlipKind => {
            q.kind = match q.kind {
                JoinKind::Inner => JoinKind::Left,
                JoinKind::Left => JoinKind::Inner,
            }
        }
        JoinTransform::ChangeOnRight => q.on_right = (q.on_right + 1) % JCOLS.len(),
        JoinTransform::AddExtraPred => q.extra = Some((0, 1)),
        JoinTransform::SwapBranches => q.swap = !q.swap,
    }
    q
}

fn join_proj_strategy() -> impl Strategy<Value = Vec<(Side, usize)>> {
    // Non-empty, deduplicated subset of {l.a, l.b, r.a, r.b}.
    let all = [(Side::L, 0), (Side::L, 1), (Side::R, 0), (Side::R, 1)];
    prop::collection::vec(0usize..4, 1..=4).prop_map(move |idxs| {
        let mut seen: Vec<(Side, usize)> = Vec::new();
        for i in idxs {
            if !seen.contains(&all[i]) {
                seen.push(all[i]);
            }
        }
        seen
    })
}

fn join_transform_strategy() -> impl Strategy<Value = JoinTransform> {
    prop_oneof![
        Just(JoinTransform::ReorderProj),
        Just(JoinTransform::Lowercase),
        Just(JoinTransform::AddComment),
        Just(JoinTransform::RenameAliases),
        Just(JoinTransform::FlipKind),
        Just(JoinTransform::ChangeOnRight),
        Just(JoinTransform::AddExtraPred),
        Just(JoinTransform::SwapBranches),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn fingerprint_soundness_joins(
        on_left in 0usize..2,
        on_right in 0usize..2,
        proj in join_proj_strategy(),
        transform in join_transform_strategy(),
    ) {
        let base = JoinQuery {
            kind: JoinKind::Inner,
            on_left,
            on_right,
            extra: None,
            proj,
            lower: false,
            comment: false,
            swap: false,
            l_alias: "l".to_string(),
            r_alias: "r".to_string(),
        };
        let transformed = apply_join(&base, &transform);

        let base_sql = base.to_sql();
        let trans_sql = transformed.to_sql();

        let fb = output_fingerprint_from_sql(&base_sql, &[])
            .unwrap_or_else(|| panic!("base did not parse: {base_sql}"));
        let ft = output_fingerprint_from_sql(&trans_sql, &[])
            .unwrap_or_else(|| panic!("transformed did not parse: {trans_sql}"));

        if fb.fingerprint == ft.fingerprint {
            let o = DuckDbRelationOracle::new();
            let rb = o.run(&base_sql)
                .unwrap_or_else(|e| panic!("base failed to run ({e}): {base_sql}"));
            let rt = o.run(&trans_sql)
                .unwrap_or_else(|e| panic!("transformed failed to run ({e}): {trans_sql}"));
            prop_assert!(
                relations_equal(&rb, &rt).is_ok(),
                "UNSOUND: equal fingerprints but DuckDB relations differ\n  base: {}\n  transformed: {}\n  diff: {:?}",
                base_sql,
                trans_sql,
                relations_equal(&rb, &rt),
            );
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn fingerprint_soundness(
        cols in proj_strategy(),
        filter in proptest::option::of((0usize..3, 0i64..10)),
        transform in transform_strategy(),
    ) {
        let proj: Vec<ProjEntry> = cols
            .iter()
            .map(|&i| ProjEntry { name: COLS[i].to_string(), expr: COLS[i].to_string() })
            .collect();
        let base = Rendered {
            cte: "data".to_string(),
            proj,
            filter: filter.map(|(i, k)| (COLS[i].to_string(), k)),
            lower: false,
            comment: false,
            derived: false,
        };
        let transformed = apply(&base, &transform);

        let base_sql = base.to_sql();
        let trans_sql = transformed.to_sql();

        let fb = output_fingerprint_from_sql(&base_sql, &[])
            .unwrap_or_else(|| panic!("base did not parse: {base_sql}"));
        let ft = output_fingerprint_from_sql(&trans_sql, &[])
            .unwrap_or_else(|| panic!("transformed did not parse: {trans_sql}"));

        if fb.fingerprint == ft.fingerprint {
            // The gate: equal fingerprints must mean equal relations.
            let o = DuckDbRelationOracle::new();
            let rb = o.run(&base_sql)
                .unwrap_or_else(|e| panic!("base failed to run ({e}): {base_sql}"));
            let rt = o.run(&trans_sql)
                .unwrap_or_else(|e| panic!("transformed failed to run ({e}): {trans_sql}"));
            prop_assert!(
                relations_equal(&rb, &rt).is_ok(),
                "UNSOUND: equal fingerprints but DuckDB relations differ\n  base: {}\n  transformed: {}\n  diff: {:?}",
                base_sql,
                trans_sql,
                relations_equal(&rb, &rt),
            );
        }
    }
}
