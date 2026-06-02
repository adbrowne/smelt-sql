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
