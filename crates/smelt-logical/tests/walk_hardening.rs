//! Killing tests for the `analysis/walk.rs` fail-closed-spine mutation
//! survivors identified by the bonus mutation campaign
//! (`docs/research/20260808-mutation-testing-maintenance-gates.md`
//! §"Bonus campaign" and its addendum). Every test here asserts through a
//! public/observable verdict surface (`model_property_vector`,
//! `batched_admission_violations`, `enumerate_scopes`,
//! `fingerprint_projection`, `model_partition_skew_excluding_self`,
//! `functional_dependency_verdict_over_vector`) — never a private
//! implementation detail.
//!
//! The per-survivor triage (which mutant each test kills, and the verdict —
//! genuine gap / provably equivalent / deferred) is recorded in the research
//! doc's campaign addendum, not duplicated here; each test's doc comment
//! names only the mutant(s) it targets.

use smelt_logical::analysis::fingerprint::{fingerprint_projection, Projection};
use smelt_logical::analysis::functional_dependency::{
    functional_dependency_verdict_over_vector, FunctionalDependencyVerdict,
};
use smelt_logical::analysis::join_shape::JoinContext;
use smelt_logical::analysis::walk::{
    batched_admission_violations, enumerate_scopes, model_partition_skew_excluding_self,
    model_property_vector, AdmissionGate, Determinism, PathSeg,
};

fn vector(sql: &str) -> smelt_logical::analysis::walk::PropertyVector {
    model_property_vector(sql, &JoinContext::new()).expect("model parses to a SELECT")
}

/// `QueryNode::has_unsupported` is the fail-closed valve every whole-tree
/// consumer (`fingerprint_projection`, `model_partition_skew_excluding_self`,
/// the monotonicity trace, `source_bounds`) falls back on when the walk
/// cannot normalize some part of the tree. Kills:
/// - `walk.rs:241` `has_unsupported -> false`
/// - `walk.rs:245` `||` -> `&&` in the `QueryNode::Select` arm
/// - `walk.rs:253` `||` -> `&&` in the `QueryNode::SetOp` arm
#[test]
fn unsupported_node_fails_closed() {
    // An unsupported FROM construct nested inside a CTE body: the outer node
    // is itself a plain `QueryNode::Select` (not `Unsupported`), so only the
    // recursive `ctes.iter().any(...)` check finds it — this is exactly what
    // the `||` -> `&&` mutant at line 245 breaks (`inputs.any` is false here,
    // so `&&` collapses the whole check to `false`).
    let cte_nested = "WITH bad AS (SELECT a FROM read_csv('data.csv')) SELECT a FROM bad";
    let verdict = fingerprint_projection(cte_nested, "sources.bad");
    match verdict {
        Projection::FullRow { reason } => assert!(
            reason.contains("cannot normalize"),
            "expected the has_unsupported fail-closed fallback reason, got: {reason}"
        ),
        Projection::Columns(cols) => panic!(
            "an unsupported construct nested in a CTE body must fall back to FullRow, \
             not a proven column set: {cols:?}"
        ),
    }

    // The same construct, but the WITH is hoisted onto a `QueryNode::SetOp`
    // (a compound query) — only reachable via the SetOp arm's `ctes.any`
    // check, which the `||` -> `&&` mutant at line 253 breaks the same way
    // (`branches.any` is false: neither branch is itself `Unsupported`).
    let setop_nested = "WITH bad AS (SELECT a FROM read_csv('data.csv')) \
         SELECT a FROM bad UNION ALL SELECT a FROM t2";
    let verdict = fingerprint_projection(setop_nested, "sources.t2");
    match verdict {
        Projection::FullRow { reason } => assert!(
            reason.contains("cannot normalize"),
            "expected the has_unsupported fail-closed fallback reason, got: {reason}"
        ),
        Projection::Columns(cols) => panic!(
            "an unsupported construct nested under a hoisted CTE of a compound query must fall \
             back to FullRow, not a proven column set: {cols:?}"
        ),
    }
}

/// `setop_kind_after` recognizes `INTERSECT`/`EXCEPT` (and their `ALL`
/// variants); grain survival across a set operation is licensed only for an
/// all-`UNION ALL` chain. Kills:
/// - `walk.rs:458` delete match arm `INTERSECT_KW`
/// - `walk.rs:459` delete match arm `EXCEPT_KW`
///
/// (When the deleted arm leaves no operator token recognised at all, the
/// whole compound degrades to a top-level `Unsupported` node — fail-closed,
/// but observably different from the correctly-recognised `SetOp` scope this
/// test pins.)
#[test]
fn intersect_except_degrade() {
    let intersect_sql = "SELECT a FROM t1 INTERSECT SELECT a FROM t2";
    let e = enumerate_scopes(intersect_sql).expect("parses");
    assert!(
        e.unsupported.is_empty(),
        "INTERSECT must be recognised as a SetOp scope, not fall back to Unsupported: {:?}",
        e.unsupported
    );
    let setop = e
        .scopes
        .iter()
        .find(|s| s.kind == smelt_logical::analysis::walk::ScopeKind::SetOp)
        .expect("a SetOp scope must be enumerated");
    assert_eq!(setop.keys, vec!["INTERSECT".to_string()]);

    let except_sql = "SELECT a FROM t1 EXCEPT SELECT a FROM t2";
    let e = enumerate_scopes(except_sql).expect("parses");
    assert!(e.unsupported.is_empty(), "EXCEPT must be recognised too");
    let setop = e
        .scopes
        .iter()
        .find(|s| s.kind == smelt_logical::analysis::walk::ScopeKind::SetOp)
        .expect("a SetOp scope must be enumerated");
    assert_eq!(setop.keys, vec!["EXCEPT".to_string()]);

    // Grain survival degrades to unkeyed the moment any arm of the chain is
    // not `UNION ALL` — pinning the `is_union_all` composition rule this
    // cluster protects.
    let mixed = "SELECT a, 'x' AS tag FROM t1 GROUP BY a \
         UNION ALL SELECT a, 'y' AS tag FROM t2 GROUP BY a \
         INTERSECT ALL SELECT a, 'z' AS tag FROM t3 GROUP BY a";
    assert!(
        vector(mixed).grain.keys.is_empty(),
        "a chain containing a non-UNION-ALL operator must degrade to unkeyed"
    );
}

/// `is_constant_literal` is the discriminator/tag candidate recognizer: a
/// bare string/number literal (optionally a typed `DATE`/`TIME`/`TIMESTAMP`/
/// `INTERVAL` literal) is constant; a column reference or anything
/// containing a function call is not. Kills:
/// - `walk.rs:2192` `is_constant_literal -> true`
/// - `walk.rs:2207` delete match arm `IDENT`
/// - `walk.rs:2209` delete `!` (the type-keyword guard)
#[test]
fn constant_literal_rejects_function_call() {
    let string_literal = vector("SELECT 'const' AS tag FROM t");
    assert!(
        string_literal
            .literal_columns
            .iter()
            .any(|(name, _)| name == "tag"),
        "a bare string literal must be recognised as a constant column"
    );

    let typed_literal = vector("SELECT DATE '2026-01-01' AS tag FROM t");
    assert!(
        typed_literal
            .literal_columns
            .iter()
            .any(|(name, _)| name == "tag"),
        "a typed DATE literal must be recognised as a constant column"
    );

    // A function call (even a zero-arg one) must never be treated as a
    // constant — kills `is_constant_literal -> true`.
    let function_call = vector("SELECT now() AS tag FROM t");
    assert!(
        !function_call
            .literal_columns
            .iter()
            .any(|(name, _)| name == "tag"),
        "a function call must never be classified as a constant literal"
    );

    // A literal combined with a genuine column reference in the same
    // expression: `saw_literal` is set by the `1`, but the trailing `qty`
    // IDENT must still reject it. Kills both the deleted `IDENT` arm (which
    // would leave `saw_literal` uncontested) and the deleted `!` on the
    // type-keyword guard (which would only reject a *type-keyword* IDENT,
    // never a genuine column-like one).
    let mixed = vector("SELECT 1 + qty AS tag FROM t");
    assert!(
        !mixed.literal_columns.iter().any(|(name, _)| name == "tag"),
        "a literal combined with a genuine column reference must not be a constant"
    );
}

/// `union_discriminated_grain`: a `UNION ALL` survives a shared key only when
/// some output position is a *pairwise-distinct* constant literal per arm.
/// Kills `walk.rs:2052` `<` -> `>` in the `branches.len() < 2` guard (a
/// two-arm degenerate-input guard that is dead for real SQL, since `from_sql`
/// never produces a `SetOp` with fewer than two branches — but a `>` variant
/// wrongly fires for three-or-more-arm chains, short-circuiting the
/// discriminator computation entirely).
#[test]
fn union_discriminator_requires_distinct_tags() {
    // Same tag in both arms: not a discriminator, no key survives.
    let collision = "SELECT id, 'x' AS tag FROM t1 GROUP BY id \
         UNION ALL SELECT id, 'x' AS tag FROM t2 GROUP BY id";
    assert!(
        vector(collision).grain.keys.is_empty(),
        "a non-distinct literal tag must not be treated as a discriminator"
    );

    // Distinct tags: the discriminator joins the shared key.
    let two_arm = "SELECT id, 'x' AS tag FROM t1 GROUP BY id \
         UNION ALL SELECT id, 'y' AS tag FROM t2 GROUP BY id";
    assert_eq!(
        vector(two_arm).grain.keys,
        vec![vec!["id".to_string(), "tag".to_string()]],
    );

    // Three arms, all pairwise-distinct tags: under the `<` -> `>` mutant,
    // `branches.len() > 2` is true for this case and the function returns
    // `branches[0].grain.clone()` directly (just `{id}`, no discriminator
    // merge) instead of running the real computation.
    let three_arm = "SELECT id, 'x' AS tag FROM t1 GROUP BY id \
         UNION ALL SELECT id, 'y' AS tag FROM t2 GROUP BY id \
         UNION ALL SELECT id, 'z' AS tag FROM t3 GROUP BY id";
    assert_eq!(
        vector(three_arm).grain.keys,
        vec![vec!["id".to_string(), "tag".to_string()]],
        "the discriminator merge must run regardless of branch count"
    );
}

/// A leaf's own transfer verdict is trivially `Default` by design (a bare
/// relation proves no properties of its own — grain/barrier/violations are
/// established by the *consuming* scope's operators). The mutants this test
/// targets are not about the leaf's own return value, but about the
/// operator-level folds that must NOT collapse a non-default child verdict
/// down to the same "as if it were a default leaf" value. Kills:
/// - `walk.rs:1892` `|=` -> `&=` in `PropertyTransfer::operator` (the
///   `has_set_op_barrier` fold through a CTE/derived input)
/// - `walk.rs:998` delete `!` in `PartitionGrainAdmission::operator` (a
///   derived-table's own violations, dropped instead of a CTE-ref's would-be
///   double-count)
/// - `walk.rs:1898` delete match arm `InputItem::Derived{alias: Some(alias), ..}`
///   (determinism/comparability reduction through a derived-table alias)
/// - `walk.rs:1913` delete `!` in the `is_distinct() && !columns.is_empty()`
///   grain guard
#[test]
fn leaf_transfer_not_default() {
    // has_set_op_barrier must propagate from a CTE input through to the
    // consuming scope, not reset to `false` (the leaf/CTE-ref default).
    let barrier_propagates =
        "WITH u AS (SELECT id FROM t1 UNION ALL SELECT id FROM t2) SELECT id FROM u";
    assert!(
        vector(barrier_propagates).has_set_op_barrier,
        "a set-op barrier inside a referenced CTE must propagate to the consuming scope"
    );

    // A derived table's own admission violation (not a CTE's) must survive
    // into the batched result.
    let derived_violation = "SELECT sub.id FROM (SELECT DISTINCT id FROM t) AS sub";
    let violations = batched_admission_violations(derived_violation, "part_col").expect("parses");
    assert!(
        violations.iter().any(|v| v.gate == AdmissionGate::Distinct),
        "a DISTINCT scope inside a derived table must trip the Distinct gate; got {violations:?}"
    );

    // Determinism must reduce through a derived table's *aliased* input —
    // the inner `NOW()` (Run) must be visible on the outer plain pass-through
    // reference, not silently reset to Clean.
    let derived_alias_determinism = "SELECT d.ts FROM (SELECT now() AS ts FROM t) AS d";
    let v = vector(derived_alias_determinism);
    let ts_level = v
        .determinism
        .iter()
        .find(|c| c.output == "ts")
        .expect("ts column present")
        .level;
    assert_eq!(
        ts_level,
        Determinism::Run,
        "a derived table's own Run-nondeterminism must propagate through its alias"
    );
}

/// `SELECT DISTINCT`'s grain is the whole projected row — but only guarded
/// by a non-empty column list. Kills `walk.rs:1913` in isolation (a plain,
/// non-derived-table DISTINCT, so the mutated guard fires on every ordinary
/// case, not just the derived-table one above).
#[test]
fn distinct_grain_uses_projected_columns() {
    let v = vector("SELECT DISTINCT a, b FROM t");
    assert_eq!(
        v.grain.keys,
        vec![vec!["a".to_string(), "b".to_string()]],
        "a DISTINCT with a non-empty column list must key on the whole projected row"
    );
}

/// An unqualified column reference resolves to a leaf only when exactly one
/// input is in scope; two-or-more must stay unresolved (ambiguous). Kills
/// `walk.rs:692` `aliases.len() == 1 -> true` in `select_lineage`.
#[test]
fn select_lineage_ambiguous_ref_not_resolved() {
    use smelt_logical::analysis::walk::{ColumnLineage, LeafInput, NodeCx, QueryTree, Transfer};

    struct LineageProbe;
    impl Transfer for LineageProbe {
        type Verdict = Vec<ColumnLineage>;
        fn leaf(&self, _leaf: &LeafInput<'_>, _cx: &NodeCx) -> Self::Verdict {
            Vec::new()
        }
        fn operator(
            &self,
            _op: &smelt_logical::analysis::walk::OpNode<'_>,
            _children: &[Self::Verdict],
            cx: &NodeCx,
        ) -> Self::Verdict {
            cx.columns.clone()
        }
    }

    // Two inputs in scope, unqualified reference: must stay unresolved.
    let sql = "SELECT id FROM t1, t2";
    let tree = QueryTree::from_sql(sql).expect("parses");
    let columns = smelt_logical::analysis::walk::walk(&tree, &LineageProbe);
    let id_col = columns
        .iter()
        .find(|c| c.output == "id")
        .expect("id column present");
    assert_eq!(
        id_col.leaf, None,
        "an unqualified reference with two inputs in scope must not resolve (ambiguous)"
    );

    // Sanity control: a single input in scope DOES resolve.
    let sql_one = "SELECT id FROM t1";
    let tree = QueryTree::from_sql(sql_one).expect("parses");
    let columns = smelt_logical::analysis::walk::walk(&tree, &LineageProbe);
    let id_col = columns
        .iter()
        .find(|c| c.output == "id")
        .expect("id column present");
    assert!(
        id_col.leaf.is_some(),
        "an unqualified reference with exactly one input in scope must resolve"
    );
}

/// `resolve_alias_source` (the determinism/comparability reduction's own
/// ambiguity guard, a separate call site from `select_lineage`'s) must also
/// refuse to reduce through an unqualified reference when more than one
/// input is in scope. Kills `walk.rs:1997` `cx.aliases.len() == 1 -> true`.
#[test]
fn determinism_not_reduced_through_ambiguous_alias() {
    // Two CTEs in scope via a comma join; the unqualified `ts` must not
    // reduce through either one's inner Run-nondeterminism.
    let sql = "WITH c1 AS (SELECT now() AS ts FROM t1), c2 AS (SELECT 1 AS ts FROM t2) \
         SELECT ts FROM c1, c2";
    let v = vector(sql);
    let ts_level = v
        .determinism
        .iter()
        .find(|c| c.output == "ts")
        .expect("ts column present")
        .level;
    assert_eq!(
        ts_level,
        Determinism::Clean,
        "an ambiguous unqualified reference (two inputs in scope) must not reduce through \
         either source's own determinism; got {ts_level:?}"
    );

    // Sanity control: a single source in scope DOES reduce.
    let sql_one = "WITH c1 AS (SELECT now() AS ts FROM t1) SELECT ts FROM c1";
    let v = vector(sql_one);
    let ts_level = v
        .determinism
        .iter()
        .find(|c| c.output == "ts")
        .expect("ts column present")
        .level;
    assert_eq!(
        ts_level,
        Determinism::Run,
        "an unambiguous unqualified reference (one input in scope) must reduce"
    );
}

/// `AdmissionViolation::path_display` renders the scope-nesting path for a
/// diagnostic message. Kills:
/// - `walk.rs:943` `path_display -> String::new()` / `-> "xyzzy".into()`
/// - `walk.rs:952` `alias.is_empty() -> true` / `-> false` (the unaliased-
///   derived-table wording)
#[test]
fn admission_violation_path_display_is_pinned() {
    // A DISTINCT inside a named derived table nested inside a CTE: exercises
    // the Cte and DerivedTable(named) path segments together.
    let named = "WITH outer_cte AS ( \
         SELECT sub.id FROM (SELECT DISTINCT id FROM t) AS sub \
     ) SELECT id FROM outer_cte";
    let violations = batched_admission_violations(named, "part_col").expect("parses");
    let v = violations
        .iter()
        .find(|v| v.gate == AdmissionGate::Distinct)
        .expect("a Distinct violation must be reported");
    assert_eq!(
        v.path,
        vec![
            PathSeg::Cte("outer_cte".to_string()),
            PathSeg::DerivedTable("sub".to_string()),
        ]
    );
    assert_eq!(
        v.path_display(),
        " (in CTE 'outer_cte' → derived table 'sub')"
    );

    // An unaliased derived table renders distinctly from a named one.
    let unaliased = "SELECT id FROM (SELECT DISTINCT id FROM t)";
    let violations = batched_admission_violations(unaliased, "part_col").expect("parses");
    let v = violations
        .iter()
        .find(|v| v.gate == AdmissionGate::Distinct)
        .expect("a Distinct violation must be reported");
    assert_eq!(v.path, vec![PathSeg::DerivedTable(String::new())]);
    assert_eq!(v.path_display(), " (in an unaliased derived table)");

    // The top-level scope's path is empty.
    let top_level = "SELECT DISTINCT id FROM t";
    let violations = batched_admission_violations(top_level, "part_col").expect("parses");
    let v = violations
        .iter()
        .find(|v| v.gate == AdmissionGate::Distinct)
        .expect("a Distinct violation must be reported");
    assert_eq!(v.path_display(), "");
}

/// `Grain::has_subset_key`: a proven key that is a subset of the declared key
/// determines every output column by augmentation. Kills
/// `walk.rs:1528` `has_subset_key -> false`.
#[test]
fn declared_fd_survives_via_subset_key() {
    let v = vector("SELECT a, b, c FROM t GROUP BY a, b");
    // Declared key {a, b, c} is a strict superset of the proven key {a, b}.
    let verdict = functional_dependency_verdict_over_vector(
        &["a".to_string(), "b".to_string(), "c".to_string()],
        "c",
        &v,
        /* declared */ false,
    );
    assert_eq!(
        verdict,
        FunctionalDependencyVerdict::Constant,
        "a proven key that is a subset of the declared key must license Constant even \
         without an explicit declaration"
    );
}

/// `model_partition_skew_excluding_self` falls back to the whole-text
/// derivation when the tree contains a construct the walk cannot normalize —
/// exact tree coverage beats fail-closed rejection here, since under-deriving
/// skew silently narrows the derived output window (see the function's own
/// doc comment). Kills `walk.rs:1487` `!tree.root.has_unsupported() -> true`
/// (which would force the walk path even when it can't see the whole tree,
/// silently losing the raw-text fallback's wider derivation).
#[test]
fn unsupported_sql_falls_back_to_whole_text_skew() {
    // A RECURSIVE CTE normalizes to a wholesale `QueryNode::Unsupported` —
    // its own text (including this Form B bound) is invisible to the walk
    // path (`OpNode::Unsupported` contributes `Skew::ZERO`, never scanning
    // its own text). The whole-text fallback, scanning the *entire* raw SQL
    // string, still finds it. Under the mutant (`!has_unsupported() -> true`
    // always), the walk path is wrongly taken and this bound is silently
    // lost — an under-derivation of skew, the dangerous direction.
    let sql = "WITH RECURSIVE bad AS ( \
         SELECT d FROM t WHERE driving BETWEEN d - INTERVAL '3 day' AND d + INTERVAL '3 day' \
     ) SELECT id FROM t2";
    let skew = model_partition_skew_excluding_self(sql, "d", None);
    assert_eq!(
        skew,
        smelt_logical::analysis::source_bounds::Skew {
            before: smelt_logical::analysis::source_bounds::Seconds::days(3),
            after: smelt_logical::analysis::source_bounds::Seconds::days(3),
        },
        "an unsupported (RECURSIVE) CTE body must not suppress the whole-text fallback's \
         derivation of a genuine Form B bound"
    );
}
