use super::*;
use crate::analysis::join_shape::JoinContext;
use crate::analysis::succession::{SuccessionContext, SuccessionVerdict};

fn scopes_of(sql: &str) -> ScopeEnumeration {
    enumerate_scopes(sql).expect("model parses to a SELECT")
}

fn find<'a>(e: &'a ScopeEnumeration, kind: ScopeKind, path: &[PathSeg]) -> Vec<&'a Scope> {
    e.scopes
        .iter()
        .filter(|s| s.kind == kind && s.path == path)
        .collect()
}

/// The pre-walk admission helpers in `rules::incremental` judge scopes
/// by iterating the outer UNION chain only
/// (`check_having_alignment_all_scopes` / `check_distinct_alignment_all_scopes`:
/// `select.having_clause()` / `select.is_distinct()` on each
/// `union_select()` link). This mirrors that chain and shows it never
/// reaches a CTE body — the hole the shared walk closes.
fn outer_chain_sees_having_or_distinct(sql: &str) -> bool {
    let parse = smelt_parser::parse(crate::types::Frontmatter::strip(sql));
    let file = smelt_parser::File::cast(parse.syntax()).expect("file");
    let mut current = file.select_stmt().expect("select stmt");
    loop {
        if current.having_clause().is_some() || current.is_distinct() {
            return true;
        }
        match current.union_select() {
            Some(next) => current = next,
            None => return false,
        }
    }
}

#[test]
fn enumerates_scopes_inside_cte_bodies() {
    let sql = "WITH dedup AS (\
             SELECT DISTINCT user_id, event_date FROM events \
             GROUP BY user_id, event_date \
             HAVING COUNT(*) > 1\
         ) \
         SELECT user_id FROM dedup";

    // The existing outer-chain walk does not see the CTE-internal
    // DISTINCT/HAVING at all — this is the hole being documented.
    assert!(
        !outer_chain_sees_having_or_distinct(sql),
        "outer-chain walk unexpectedly sees CTE-internal scopes"
    );

    let e = scopes_of(sql);
    let cte_path = vec![PathSeg::Cte("dedup".to_string())];
    assert_eq!(
        find(&e, ScopeKind::Distinct, &cte_path).len(),
        1,
        "DISTINCT inside the CTE body must be enumerated; got: {:?}",
        e.scopes
    );
    let having = find(&e, ScopeKind::Having, &cte_path);
    assert_eq!(
        having.len(),
        1,
        "HAVING inside the CTE body must be enumerated; got: {:?}",
        e.scopes
    );
    let group_by = find(&e, ScopeKind::GroupBy, &cte_path);
    assert_eq!(group_by.len(), 1);
    assert_eq!(group_by[0].keys, vec!["user_id", "event_date"]);
    assert!(
        e.unsupported.is_empty(),
        "nothing unrecognised here: {:?}",
        e.unsupported
    );
}

#[test]
fn expression_position_distinct_is_judged_by_admission() {
    // A `SELECT DISTINCT` in an expression-position scalar subquery is the
    // SC-7 cross-partition-dedup hazard one nesting level down: it dedups
    // `profiles` rows across partitions, unaligned to the model's `d`
    // partition. It is not a walk node, so it must be judged in the owning
    // scope's region — the admission gate must flag it, not silently pass.
    let sql = "SELECT e.d, e.user_id, \
                   (SELECT DISTINCT tier FROM smelt.sources.profiles p \
                     WHERE p.user_id = e.user_id) AS tier \
                   FROM smelt.sources.events e";
    let violations = batched_admission_violations(sql, "d").expect("model parses");
    assert!(
        violations.iter().any(|v| v.gate == AdmissionGate::Distinct),
        "expression-position SELECT DISTINCT must trip the Distinct gate; got {violations:?}"
    );
}

#[test]
fn expression_position_aligned_distinct_is_admitted() {
    // An expression-position DISTINCT whose key includes the partition
    // column is partition-local — it must NOT trip the gate (no
    // over-refusal of a legitimately-aligned dedup).
    let sql = "SELECT e.d, \
                   (SELECT DISTINCT d FROM smelt.sources.profiles p \
                     WHERE p.user_id = e.user_id) AS d2 \
                   FROM smelt.sources.events e";
    let violations = batched_admission_violations(sql, "d").expect("model parses");
    assert!(
        !violations.iter().any(|v| v.gate == AdmissionGate::Distinct),
        "an aligned expression-position DISTINCT must be admitted; got {violations:?}"
    );
}

#[test]
fn enumerates_set_op_branch_scopes_per_branch() {
    let sql = "SELECT event_date, COUNT(*) AS cnt FROM events_a GROUP BY event_date \
             UNION ALL \
             SELECT event_date, COUNT(*) AS cnt FROM events_b GROUP BY event_date";
    let e = scopes_of(sql);

    let b0 = find(&e, ScopeKind::GroupBy, &[PathSeg::SetOpBranch(0)]);
    let b1 = find(&e, ScopeKind::GroupBy, &[PathSeg::SetOpBranch(1)]);
    assert_eq!(
        (b0.len(), b1.len()),
        (1, 1),
        "each branch's GROUP BY is its own scope with its own path; got: {:?}",
        e.scopes
    );

    let setop = find(&e, ScopeKind::SetOp, &[]);
    assert_eq!(setop.len(), 1);
    assert_eq!(setop[0].keys, vec!["UNION ALL"]);
    assert!(e.unsupported.is_empty());
}

#[test]
fn derived_table_and_nested_cte_scopes() {
    let sql = "WITH base AS (\
             SELECT user_id, event_date FROM events GROUP BY user_id, event_date\
         ), daily AS (\
             SELECT event_date FROM base GROUP BY event_date\
         ) \
         SELECT d.event_date FROM (\
             SELECT event_date FROM daily GROUP BY event_date\
         ) d";
    let e = scopes_of(sql);

    let base = find(&e, ScopeKind::GroupBy, &[PathSeg::Cte("base".to_string())]);
    let daily = find(&e, ScopeKind::GroupBy, &[PathSeg::Cte("daily".to_string())]);
    let derived = find(
        &e,
        ScopeKind::GroupBy,
        &[PathSeg::DerivedTable("d".to_string())],
    );
    assert_eq!(
        (base.len(), daily.len(), derived.len()),
        (1, 1, 1),
        "CTE-referencing-CTE and subquery-in-FROM scopes all visited once; got: {:?}",
        e.scopes
    );

    // Dependency order is stable: base (defined first) before daily
    // (which references it), both before the derived table's scope.
    let pos = |path: &[PathSeg]| {
        e.scopes
            .iter()
            .position(|s| s.kind == ScopeKind::GroupBy && s.path == path)
            .expect("scope present")
    };
    let base_pos = pos(&[PathSeg::Cte("base".to_string())]);
    let daily_pos = pos(&[PathSeg::Cte("daily".to_string())]);
    let derived_pos = pos(&[PathSeg::DerivedTable("d".to_string())]);
    assert!(
        base_pos < daily_pos && daily_pos < derived_pos,
        "dependency order must be stable: base < daily < derived; got {:?}",
        e.scopes
    );
    assert!(e.unsupported.is_empty());
}

#[test]
fn alias_resolution_through_cte_rename() {
    // A column renamed through a CTE projection resolves to its source
    // leaf in the consuming node's context.
    let sql = "WITH c AS (SELECT user_id AS uid FROM events) SELECT uid FROM c";

    struct LineageProbe;
    impl Transfer for LineageProbe {
        type Verdict = Vec<ColumnLineage>;
        fn leaf(&self, _leaf: &LeafInput<'_>, _cx: &NodeCx) -> Self::Verdict {
            Vec::new()
        }
        fn operator(
            &self,
            _op: &OpNode<'_>,
            _children: &[Self::Verdict],
            cx: &NodeCx,
        ) -> Self::Verdict {
            cx.columns.clone()
        }
    }

    let tree = QueryTree::from_sql(sql).expect("parses");
    let columns = walk(&tree, &LineageProbe);
    assert_eq!(columns.len(), 1, "one projected column; got {columns:?}");
    assert_eq!(columns[0].output, "uid");
    assert_eq!(
        columns[0].leaf,
        Some(LeafColumn {
            relation: "events".to_string(),
            column: "user_id".to_string(),
        }),
        "uid must chase through the CTE rename to events.user_id"
    );
}

#[test]
fn unrecognised_from_construct_is_fail_loud() {
    // A table function in FROM is not yet a recognised leaf: the walk
    // must surface an explicit Unsupported entry, never an empty
    // enumeration that consumers could mistake for "no scopes, admit".
    let sql = "SELECT a FROM read_csv('data.csv')";
    let e = scopes_of(sql);
    assert!(
        !e.unsupported.is_empty(),
        "table function in FROM must yield an Unsupported entry"
    );
}

/// The walk-composed skew fold: a sessions-shaped model (the Form B
/// relation lives in the outer scope, below a CTE) composes to a
/// symmetric 1-day skew; an identity model composes to zero; and a
/// relation buried inside a CTE body still surfaces at the root (the
/// union fold across walk nodes).
#[test]
fn skew_fold_composes_across_scopes() {
    use super::super::source_bounds::{Seconds, Skew};

    let sessions_shaped = "WITH sessionized AS (\
             SELECT user_id, event_date, session_start_date FROM smelt.silver.events\
         ) \
         SELECT * FROM sessionized \
         WHERE event_date BETWEEN session_start_date - INTERVAL '1 day' \
             AND session_start_date + INTERVAL '1 day'";
    let skew = model_partition_skew(sessions_shaped, "session_start_date");
    assert_eq!(
        skew,
        Skew {
            before: Seconds::days(1),
            after: Seconds::days(1),
        },
        "sessions-shaped model must compose a symmetric 1-day skew"
    );

    let identity = "SELECT event_date, COUNT(*) AS n \
             FROM smelt.silver.events GROUP BY event_date";
    assert_eq!(
        model_partition_skew(identity, "event_date"),
        Skew::ZERO,
        "identity model must compose zero skew"
    );

    // The Form B relation inside a CTE body (its own walk node) must
    // reach the root verdict via the union fold.
    let nested = "WITH capped AS (\
             SELECT * FROM smelt.silver.events \
             WHERE event_date BETWEEN session_start_date - INTERVAL '2 days' \
                 AND session_start_date + INTERVAL '1 day'\
         ) \
         SELECT * FROM capped";
    let skew = model_partition_skew(nested, "session_start_date");
    assert_eq!(
        skew,
        Skew {
            before: Seconds::days(2),
            after: Seconds::days(1),
        },
        "a CTE-scope Form B relation must surface in the root verdict"
    );
}

#[test]
fn reach_series_adds_parallel_maxes() {
    use super::super::source_bounds::{derive_model_bounds, BoundContext, BoundResult, Seconds};

    // Stacked frames across a CTE boundary compose in SERIES: an output
    // row reads 3d of s7 values, each of which reads 7d of source rows —
    // true backward reach 10d, not max(7, 3) = 7.
    let stacked = "WITH seven AS (\
             SELECT d, SUM(v) OVER (ORDER BY d RANGE BETWEEN INTERVAL '7 days' PRECEDING AND CURRENT ROW) AS s7 \
             FROM smelt.sources.metrics\
         ) \
         SELECT d, MAX(s7) OVER (ORDER BY d RANGE BETWEEN INTERVAL '3 days' PRECEDING AND CURRENT ROW) AS m3 \
         FROM seven";
    let ctx = BoundContext::new().with_source("sources.metrics", "d");
    let bounds = derive_model_bounds(stacked, &ctx);
    assert_eq!(
        bounds.get("sources.metrics"),
        Some(&BoundResult::Bounded {
            source_partition_col: "d".to_string(),
            before: Seconds::days(10),
            after: Seconds::ZERO,
        }),
        "stacked frames must series-add (7d + 3d = 10d)"
    );

    // Set-operation arms are PARALLEL: the source is read independently
    // by each arm, so the reach is the max across arms, not the sum.
    let unioned = "SELECT d, SUM(v) OVER (ORDER BY d RANGE BETWEEN INTERVAL '7 days' PRECEDING AND CURRENT ROW) AS x \
             FROM smelt.sources.metrics \
             UNION ALL \
             SELECT d, SUM(v) OVER (ORDER BY d RANGE BETWEEN INTERVAL '3 days' PRECEDING AND CURRENT ROW) AS x \
             FROM smelt.sources.metrics";
    let bounds = derive_model_bounds(unioned, &ctx);
    assert_eq!(
        bounds.get("sources.metrics"),
        Some(&BoundResult::Bounded {
            source_partition_col: "d".to_string(),
            before: Seconds::days(7),
            after: Seconds::ZERO,
        }),
        "set-op arms must parallel-max (max(7d, 3d) = 7d)"
    );

    // A symbolic (month/year) offset anywhere on the series path is
    // absorbing: the source's bound is NotDerivable, never approximated.
    let symbolic = "WITH seven AS (\
             SELECT d, SUM(v) OVER (ORDER BY d RANGE BETWEEN INTERVAL '1 month' PRECEDING AND CURRENT ROW) AS s7 \
             FROM smelt.sources.metrics\
         ) \
         SELECT d, MAX(s7) OVER (ORDER BY d RANGE BETWEEN INTERVAL '3 days' PRECEDING AND CURRENT ROW) AS m3 \
         FROM seven";
    let bounds = derive_model_bounds(symbolic, &ctx);
    assert_eq!(
        bounds.get("sources.metrics"),
        Some(&BoundResult::NotDerivable),
        "a symbolic offset in a series position must absorb to NotDerivable"
    );
}

/// Set-operation arms carry a per-branch trace VECTOR: arms anchored to
/// different sources each keep their own verdict — there is no collapsed
/// single-source reduction — and a `StaticSeed` arm keeps its per-branch
/// refusal (the consumer's per-branch policy refuses that branch's push),
/// never averaged away.
#[test]
fn set_op_branch_trace_vector() {
    use super::super::monotonicity::{trace_event_time_composed, ComposedTrace, EventTimeTrace};
    use super::super::source_bounds::BoundContext;

    let ctx = BoundContext::new()
        .with_source("sources.a", "a_ts")
        .with_source("sources.b", "b_ts");

    let sql = "SELECT a_ts AS event_time FROM smelt.sources.a \
                   UNION ALL \
                   SELECT b_ts AS event_time FROM smelt.sources.b";
    match trace_event_time_composed(sql, "event_time", &ctx, false) {
        Some(ComposedTrace::Branches(traces)) => {
            assert_eq!(traces.len(), 2, "one trace per arm; got {traces:?}");
            match (&traces[0], &traces[1]) {
                (
                    EventTimeTrace::Traceable { source: s0, .. },
                    EventTimeTrace::Traceable { source: s1, .. },
                ) => {
                    assert_eq!(s0, "sources.a");
                    assert_eq!(s1, "sources.b");
                }
                other => {
                    panic!("each branch must trace to its own source, got {other:?}")
                }
            }
        }
        other => panic!("expected a per-branch trace vector, got {other:?}"),
    }

    let sql = "SELECT a_ts AS event_time FROM smelt.sources.a \
                   UNION ALL \
                   SELECT NULL AS event_time FROM smelt.sources.b";
    match trace_event_time_composed(sql, "event_time", &ctx, false) {
        Some(ComposedTrace::Branches(traces)) => {
            assert_eq!(traces.len(), 2);
            assert!(matches!(traces[0], EventTimeTrace::Traceable { .. }));
            assert!(
                matches!(traces[1], EventTimeTrace::StaticSeed { .. }),
                "the seed branch must keep its own refusal in the vector, got {:?}",
                traces[1]
            );
        }
        other => {
            panic!("expected a per-branch vector with the seed branch preserved, got {other:?}")
        }
    }
}

#[test]
fn chained_join_bands_add_along_path() {
    use super::super::source_bounds::{derive_model_bounds, BoundContext, BoundResult, Seconds};

    // Chained interval-join bands: b within 1d of a, c within 2d of b —
    // source c's reach relative to the run window is 3d. Structurally the
    // chain nests (the inner hop is a CTE the outer hop joins), so the
    // series-add composes the bands along the path.
    let sql = "WITH ab AS (\
             SELECT b.d AS d, b.v AS v \
             FROM smelt.sources.a a \
             JOIN smelt.sources.b b ON b.d >= a.d - INTERVAL '1 day' AND b.d <= a.d\
         ) \
         SELECT c.d, c.v \
         FROM ab \
         JOIN smelt.sources.c c ON c.d >= ab.d - INTERVAL '2 days' AND c.d <= ab.d";
    let ctx = BoundContext::new()
        .with_source("sources.a", "d")
        .with_source("sources.b", "d")
        .with_source("sources.c", "d");
    let bounds = derive_model_bounds(sql, &ctx);
    assert_eq!(
        bounds.get("sources.c"),
        Some(&BoundResult::Bounded {
            source_partition_col: "d".to_string(),
            before: Seconds::days(3),
            after: Seconds::ZERO,
        }),
        "the far source's bands must add along the join path (1d + 2d = 3d)"
    );
    // The nearer sources stay bounded (the composition may widen them —
    // a wider scan is sound — but must never lose their bound).
    for src in ["sources.a", "sources.b"] {
        assert!(
            matches!(bounds.get(src), Some(BoundResult::Bounded { .. })),
            "{src} must stay Bounded; got {:?}",
            bounds.get(src)
        );
    }
}

#[test]
fn expression_subquery_reference_site_keeps_flat_scan_floor() {
    use super::super::source_bounds::{derive_model_bounds, BoundContext, BoundResult, Seconds};

    // `metrics` is referenced twice: once as a plain FROM leaf (the outer
    // JOIN, zero margin) and once inside an expression-position scalar
    // subquery carrying a 30-day lookback band. The subquery is not a walk
    // node, so the leaf-path walk verdict only carries the zero-margin FROM
    // reach; the flat-scan floor must widen it back to 30 days so the
    // injected scan filter covers the subquery's reference site.
    let sql = "WITH agg AS (\
             SELECT u.d, \
                    (SELECT MAX(m.v) FROM smelt.sources.metrics m \
                      WHERE m.d >= u.d - INTERVAL '30 days' AND m.d <= u.d) AS peak \
             FROM smelt.sources.activity u\
         ) \
         SELECT r.d, r.v, a.peak \
         FROM smelt.sources.metrics r JOIN agg a ON a.d = r.d";
    let ctx = BoundContext::new()
        .with_source("sources.metrics", "d")
        .with_source("sources.activity", "d");
    let bounds = derive_model_bounds(sql, &ctx);
    match bounds.get("sources.metrics") {
        Some(BoundResult::Bounded { before, .. }) => {
            assert!(
                *before >= Seconds::days(30),
                "metrics scan must cover the subquery's 30-day band; got before={:?}",
                before
            );
        }
        other => panic!("expected metrics Bounded with >=30d before, got {other:?}"),
    }
}

// ===== Property-vector transfer functions =====

fn vector_of(sql: &str) -> PropertyVector {
    model_property_vector(sql, &JoinContext::new()).expect("model parses to a SELECT")
}

fn keyset(cols: &[&str]) -> std::collections::BTreeSet<String> {
    cols.iter().map(|c| c.to_ascii_lowercase()).collect()
}

#[test]
fn group_by_establishes_grain_and_fds() {
    // The GROUP BY factory: the grouping key uniquely identifies an output
    // row, so `customer_id → every output column` by construction (§3.5).
    let v = vector_of("SELECT customer_id, SUM(amount) AS total FROM orders GROUP BY customer_id");

    assert!(
        v.grain.keys.iter().any(
            |k| keyset(&k.iter().map(String::as_str).collect::<Vec<_>>())
                == keyset(&["customer_id"])
        ),
        "GROUP BY customer_id must establish [customer_id] as a grain key; got {:?}",
        v.grain
    );
    assert!(
        v.fds
            .iter()
            .any(|fd| fd.key == vec!["customer_id".to_string()] && fd.determines == "total"),
        "the factory must carry customer_id → total; got {:?}",
        v.fds
    );
}

#[test]
fn union_all_drops_grain_and_fds_unless_discriminated() {
    // Both arms keyed by [customer_id], but the union has no discriminator
    // — the same customer_id may appear in both arms, so the union is
    // unkeyed (§3.8: FD/grain destroyed by a bare UNION ALL).
    let undiscriminated = vector_of(
        "SELECT customer_id FROM crm_a GROUP BY customer_id \
             UNION ALL \
             SELECT customer_id FROM crm_b GROUP BY customer_id",
    );
    assert!(
        undiscriminated.grain.keys.is_empty(),
        "a bare UNION ALL of two keyed arms is unkeyed; got {:?}",
        undiscriminated.grain
    );
    assert!(
        undiscriminated.has_set_op_barrier,
        "the union node must record its FD barrier"
    );

    // A distinct literal tag column per arm, added to the key, makes the
    // arms provably disjoint — (src, customer_id) survives as a key.
    let discriminated = vector_of(
        "SELECT 'a' AS src, customer_id FROM crm_a GROUP BY customer_id \
             UNION ALL \
             SELECT 'b' AS src, customer_id FROM crm_b GROUP BY customer_id",
    );
    assert!(
        discriminated.grain.keys.iter().any(|k| keyset(
            &k.iter().map(String::as_str).collect::<Vec<_>>()
        ) == keyset(&["src", "customer_id"])),
        "a literal discriminator in the key preserves the union key; got {:?}",
        discriminated.grain
    );
}

#[test]
fn determinism_predicate_registered_as_leaf() {
    // clean ∪ clean = clean across a UNION ALL (columnar union lub).
    let clean = vector_of("SELECT user_id FROM events_a UNION ALL SELECT user_id FROM events_b");
    assert_eq!(
        clean
            .determinism
            .iter()
            .find(|d| d.output == "user_id")
            .map(|d| d.level),
        Some(Determinism::Clean),
        "clean ∪ clean must stay clean; got {:?}",
        clean.determinism
    );

    // A row-nondeterministic function taints its column (leaf predicate).
    let row = vector_of("SELECT random() AS r FROM t");
    assert_eq!(
        row.determinism
            .iter()
            .find(|d| d.output == "r")
            .map(|d| d.level),
        Some(Determinism::Row),
        "random() must classify Row; got {:?}",
        row.determinism
    );

    // A run-deterministic clock is a per-run constant, not row-tainted.
    let run = vector_of("SELECT now() AS t FROM src");
    assert_eq!(
        run.determinism
            .iter()
            .find(|d| d.output == "t")
            .map(|d| d.level),
        Some(Determinism::Run),
        "now() must classify Run; got {:?}",
        run.determinism
    );
}

// ===== Change-comparability lattice fold (P3) =====

fn comparability_of<'a>(v: &'a PropertyVector, output: &str) -> Option<&'a Comparability> {
    v.comparability
        .iter()
        .find(|c| c.output == output)
        .map(|c| &c.comparability)
}

#[test]
fn comparability_leaf_classification() {
    // A plain aggregate column is a pure function of processed inputs —
    // comparable across runs.
    let agg =
        vector_of("SELECT customer_id, SUM(amount) AS total FROM orders GROUP BY customer_id");
    assert_eq!(
        comparability_of(&agg, "total"),
        Some(&Comparability::Comparable),
        "a plain SUM aggregate must be Comparable; got {:?}",
        agg.comparability
    );

    // A run-pinned clock is comparable *within* a run but not *across*
    // runs — Incomparable.
    let run = vector_of("SELECT now() AS t FROM src");
    assert_eq!(
        comparability_of(&run, "t"),
        Some(&Comparability::Incomparable),
        "now() must be Incomparable across runs; got {:?}",
        run.comparability
    );

    // A row-nondeterministic value is unpinnable — Incomparable.
    let row = vector_of("SELECT random() AS r FROM t");
    assert_eq!(
        comparability_of(&row, "r"),
        Some(&Comparability::Incomparable),
        "random() must be Incomparable; got {:?}",
        row.comparability
    );

    // An unrecognised (opaque) function call is not known to be a pure
    // deterministic function of its inputs — fail-closed to Incomparable,
    // never a default Comparable.
    let opaque = vector_of("SELECT my_opaque_udf(amount) AS y FROM orders");
    assert_eq!(
        comparability_of(&opaque, "y"),
        Some(&Comparability::Incomparable),
        "an unrecognised function call must fail-closed to Incomparable; got {:?}",
        opaque.comparability
    );
}

#[test]
fn comparability_folds_through_cte() {
    // A NOW()-tainted CTE column, read plainly by the outer scope, must
    // still be Incomparable — comparability composes through the walk's
    // CTE reduction, not re-derived from the outer scope's own text.
    let v = vector_of(
        "WITH staged AS (SELECT now() AS t, customer_id FROM src) \
             SELECT t, customer_id FROM staged",
    );
    assert_eq!(
        comparability_of(&v, "t"),
        Some(&Comparability::Incomparable),
        "a NOW()-tainted CTE column must stay Incomparable through the outer read; got {:?}",
        v.comparability
    );
    assert_eq!(
        comparability_of(&v, "customer_id"),
        Some(&Comparability::Comparable),
        "a plain passthrough CTE column must stay Comparable; got {:?}",
        v.comparability
    );
}

#[test]
fn comparability_union_lub() {
    // A column comparable in one arm and incomparable in the other folds
    // Incomparable (the union operator rule = lub, same shape as
    // determinism's per-position max).
    let v = vector_of(
        "SELECT customer_id AS id FROM crm_a \
             UNION ALL \
             SELECT now() AS id FROM crm_b",
    );
    assert_eq!(
        comparability_of(&v, "id"),
        Some(&Comparability::Incomparable),
        "one Incomparable arm must dominate the union; got {:?}",
        v.comparability
    );
}

fn succession_fixture_ctx() -> SuccessionContext {
    use crate::analysis::input_delta::MutationProfile;
    SuccessionContext {
        source_name: "raw.customer_changes".to_string(),
        mutation_profile: Some(MutationProfile::AppendOnly),
        event_time_column: Some("changed_at".to_string()),
        not_null_columns: ["customer_id", "changed_at"]
            .into_iter()
            .map(String::from)
            .collect(),
    }
}

#[test]
fn walk_invokes_succession_leaf() {
    use crate::analysis::succession::classify_keyed_succession;

    let sql = "SELECT customer_id, changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes";
    let ctx = succession_fixture_ctx();
    let tree = QueryTree::from_sql(sql).expect("sql parses");
    let QueryNode::Select(node) = &tree.root else {
        panic!("expected a top-level SELECT scope");
    };
    assert_eq!(
        model_keyed_succession(&tree, &ctx),
        classify_keyed_succession(node, &ctx),
        "the walk entry must return exactly the classifier's own verdict for the top scope"
    );
}

#[test]
fn walk_refuses_a_succession_shape_nested_in_a_union_arm() {
    let sql = "SELECT customer_id, changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.customer_changes \
                    UNION ALL \
                    SELECT customer_id, changed_at, \
                    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_ts \
                    FROM smelt.raw.other_changes";
    let ctx = succession_fixture_ctx();
    let tree = QueryTree::from_sql(sql).expect("sql parses");
    assert!(matches!(
        model_keyed_succession(&tree, &ctx),
        SuccessionVerdict::NotSuccession { .. }
    ));
}
