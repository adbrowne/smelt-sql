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

// ===========================================================================
// §5.5 axes — "same printed type does not imply same rows".
//
// Generates queries that exercise the value-affecting dimensions §5.5 flags:
//   - row-affecting tail clauses: LIMIT / OFFSET,
//   - decimal scale (CAST AS DECIMAL(p, s)),
//   - nullability (NULLIF) and DISTINCT.
// The invariant is the same: equal fingerprint ⇒ equal relation.
//
// Determinism is a precondition for DuckDB being a stable oracle, so a sliced
// query (LIMIT/OFFSET) is always given a total ORDER BY over its projected
// columns. A bare LIMIT/OFFSET *without* a total ORDER BY is itself
// non-deterministic — DuckDB returns an arbitrary row, and two runs of the
// identical query can disagree (this generator demonstrated it). That is the
// §5.5 "unbounded-without-total-order" hazard, on plain pagination rather than
// `random()`/`now()`. Inline non-determinism in general (`now()`, `random()`,
// unordered slices) is therefore NOT asserted here: such a query cannot satisfy
// "fp-equal ⇒ rows-equal" at all, so the sound answer is a determinism model
// that marks the model non-reusable (accept-current / assert-deterministic),
// not a canonicaliser change. That gap is tracked in the research doc.
// ===========================================================================

#[derive(Debug, Clone, Copy)]
enum Wrap {
    Plain,
    /// `CAST(col AS DECIMAL(18, scale))`.
    DecimalScale(u8),
    /// `NULLIF(col, k)`.
    NullIf(i64),
}

#[derive(Debug, Clone)]
struct S5Query {
    proj: Vec<(usize, Wrap)>, // (column index, expression wrapper)
    distinct: bool,
    limit: Option<u32>,
    offset: Option<u32>,
}

#[derive(Debug, Clone)]
enum S5Transform {
    ReorderProj,
    SetWrap(usize, Wrap),
    ToggleDistinct,
    SetLimit(u32),
    ClearLimit,
    SetOffset(u32),
}

impl S5Query {
    fn render_col((col, wrap): (usize, Wrap)) -> String {
        let c = COLS[col];
        let expr = match wrap {
            Wrap::Plain => c.to_string(),
            Wrap::DecimalScale(s) => format!("CAST({c} AS DECIMAL(18, {s}))"),
            Wrap::NullIf(k) => format!("NULLIF({c}, {k})"),
        };
        format!("{expr} AS {c}")
    }

    fn to_sql(&self) -> String {
        let cols: Vec<String> = self.proj.iter().copied().map(Self::render_col).collect();
        let distinct = if self.distinct { "DISTINCT " } else { "" };
        let limit = match self.limit {
            Some(n) => format!(" LIMIT {n}"),
            None => String::new(),
        };
        let offset = match self.offset {
            Some(n) => format!(" OFFSET {n}"),
            None => String::new(),
        };
        // A sliced query needs a total order to be deterministic; order over the
        // projected output names (always valid, incl. under DISTINCT).
        let order = if self.limit.is_some() || self.offset.is_some() {
            let names: Vec<&str> = self.proj.iter().map(|&(c, _)| COLS[c]).collect();
            format!(" ORDER BY {}", names.join(", "))
        } else {
            String::new()
        };
        format!(
            "SELECT {distinct}{cols} FROM ({SEED_BODY}) AS t{order}{limit}{offset}",
            cols = cols.join(", "),
        )
    }
}

fn apply_s5(base: &S5Query, t: &S5Transform) -> S5Query {
    let mut q = base.clone();
    match t {
        S5Transform::ReorderProj => {
            if q.proj.len() > 1 {
                q.proj.rotate_left(1);
            }
        }
        S5Transform::SetWrap(i, w) => {
            let idx = i % q.proj.len();
            q.proj[idx].1 = *w;
        }
        S5Transform::ToggleDistinct => q.distinct = !q.distinct,
        S5Transform::SetLimit(n) => q.limit = Some(*n),
        S5Transform::ClearLimit => q.limit = None,
        S5Transform::SetOffset(n) => q.offset = Some(*n),
    }
    q
}

fn wrap_strategy() -> impl Strategy<Value = Wrap> {
    prop_oneof![
        Just(Wrap::Plain),
        (0u8..4).prop_map(Wrap::DecimalScale),
        (0i64..4).prop_map(Wrap::NullIf),
    ]
}

fn s5_transform_strategy() -> impl Strategy<Value = S5Transform> {
    prop_oneof![
        Just(S5Transform::ReorderProj),
        (0usize..3, wrap_strategy()).prop_map(|(i, w)| S5Transform::SetWrap(i, w)),
        Just(S5Transform::ToggleDistinct),
        (0u32..3).prop_map(S5Transform::SetLimit),
        Just(S5Transform::ClearLimit),
        (0u32..3).prop_map(S5Transform::SetOffset),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn fingerprint_soundness_s5_axes(
        cols in proj_strategy(),
        distinct in any::<bool>(),
        limit in proptest::option::of(0u32..3),
        offset in proptest::option::of(0u32..3),
        transform in s5_transform_strategy(),
    ) {
        let base = S5Query {
            proj: cols.iter().map(|&i| (i, Wrap::Plain)).collect(),
            distinct,
            limit,
            offset,
        };
        let transformed = apply_s5(&base, &transform);

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
