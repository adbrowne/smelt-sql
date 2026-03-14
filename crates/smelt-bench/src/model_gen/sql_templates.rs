use super::graph_builder::ModelSpec;
use rand::prelude::*;
use rand_chacha::ChaChaRng;

/// Column names used across templates for realistic SQL.
const COLUMNS: &[&str] = &[
    "user_id",
    "event_time",
    "created_at",
    "updated_at",
    "amount",
    "category",
    "status",
    "event_type",
    "session_id",
    "product_id",
    "order_id",
    "quantity",
    "price",
    "discount",
    "revenue",
    "cost",
    "profit",
    "country",
    "region",
    "platform",
    "device_type",
    "browser",
    "os_name",
    "referrer",
    "campaign_id",
    "channel",
    "is_active",
    "is_verified",
    "score",
    "rating",
    "duration_seconds",
    "page_path",
    "ip_address",
    "email_domain",
    "plan_type",
    "tier",
    "segment",
    "cohort_date",
    "event_date",
    "transaction_id",
];

/// Aggregate functions used in templates.
const AGGREGATES: &[&str] = &[
    "COUNT(*)",
    "SUM(amount)",
    "AVG(amount)",
    "MIN(created_at)",
    "MAX(created_at)",
    "COUNT(DISTINCT user_id)",
    "SUM(quantity)",
    "AVG(price)",
    "SUM(revenue)",
    "AVG(duration_seconds)",
];

/// WHERE clause conditions.
const CONDITIONS: &[&str] = &[
    "status = 'active'",
    "event_type = 'purchase'",
    "amount > 0",
    "is_active = true",
    "created_at >= '2024-01-01'",
    "category IS NOT NULL",
    "country = 'US'",
    "platform = 'web'",
    "quantity > 0",
    "score >= 50",
];

/// YAML frontmatter for incremental models.
fn frontmatter() -> &'static str {
    "---\nmaterialization: table\nincremental:\n  enabled: true\n  partition_column: event_date\n---\n"
}

/// Generate SQL content for a model spec using a randomly selected template.
pub fn generate_sql(rng: &mut ChaChaRng, spec: &ModelSpec) -> String {
    let template_idx = rng.gen_range(0..12);
    let ref_strs: Vec<String> = spec
        .dependencies
        .iter()
        .map(|d| format!("smelt.ref('{}')", d))
        .collect();

    match template_idx {
        0 => simple_select(rng, &ref_strs),
        1 => aggregation(rng, &ref_strs),
        2 => join_two(rng, &ref_strs),
        3 => join_three(rng, &ref_strs),
        4 => window_function(rng, &ref_strs),
        5 => cte_pattern(rng, &ref_strs),
        6 => union_all(rng, &ref_strs),
        7 => case_expression(rng, &ref_strs),
        8 => subquery_filter(rng, &ref_strs),
        9 => date_aggregation(rng, &ref_strs),
        10 => having_filter(rng, &ref_strs),
        11 => complex_mixed(rng, &ref_strs),
        _ => simple_select(rng, &ref_strs),
    }
}

fn pick_columns(rng: &mut ChaChaRng, count: usize) -> Vec<&'static str> {
    let mut indices: Vec<usize> = (0..COLUMNS.len()).collect();
    indices.shuffle(rng);
    indices.truncate(count);
    indices.into_iter().map(|i| COLUMNS[i]).collect()
}

fn pick_aggs(rng: &mut ChaChaRng, count: usize) -> Vec<&'static str> {
    let mut indices: Vec<usize> = (0..AGGREGATES.len()).collect();
    indices.shuffle(rng);
    indices.truncate(count);
    indices.into_iter().map(|i| AGGREGATES[i]).collect()
}

fn pick_condition(rng: &mut ChaChaRng) -> &'static str {
    CONDITIONS[rng.gen_range(0..CONDITIONS.len())]
}

/// 1. Simple SELECT with WHERE filter.
fn simple_select(rng: &mut ChaChaRng, refs: &[String]) -> String {
    let cols = pick_columns(rng, 4);
    let cond = pick_condition(rng);
    let source = &refs[0];
    format!(
        "{fm}SELECT\n    {c1},\n    {c2},\n    {c3},\n    {c4}\nFROM {source}\nWHERE {cond}\n",
        fm = frontmatter(),
        c1 = cols[0],
        c2 = cols[1],
        c3 = cols[2],
        c4 = cols[3],
    )
}

/// 2. GROUP BY with aggregate functions.
fn aggregation(rng: &mut ChaChaRng, refs: &[String]) -> String {
    let group_col = pick_columns(rng, 1)[0];
    let num_aggs = rng.gen_range(2..=5);
    let aggs = pick_aggs(rng, num_aggs);
    let agg_lines: Vec<String> = aggs
        .iter()
        .enumerate()
        .map(|(i, a)| format!("    {} AS agg_{}", a, i))
        .collect();
    let source = &refs[0];
    format!(
        "{fm}SELECT\n    {group_col},\n{aggs}\nFROM {source}\nGROUP BY {group_col}\n",
        fm = frontmatter(),
        aggs = agg_lines.join(",\n"),
    )
}

/// 3. INNER/LEFT JOIN of two refs.
fn join_two(rng: &mut ChaChaRng, refs: &[String]) -> String {
    let cols = pick_columns(rng, 3);
    let join_type = if rng.gen_bool(0.5) { "INNER" } else { "LEFT" };
    let (r1, r2) = if refs.len() >= 2 {
        (&refs[0], &refs[1])
    } else {
        (&refs[0], &refs[0])
    };
    format!(
        "{fm}SELECT\n    a.{c1},\n    a.{c2},\n    b.{c3}\nFROM {r1} a\n{jt} JOIN {r2} b ON a.user_id = b.user_id\n",
        fm = frontmatter(),
        c1 = cols[0],
        c2 = cols[1],
        c3 = cols[2],
        jt = join_type,
    )
}

/// 4. Three-way join.
fn join_three(rng: &mut ChaChaRng, refs: &[String]) -> String {
    let cols = pick_columns(rng, 4);
    let (r1, r2, r3) = match refs.len() {
        1 => (&refs[0], &refs[0], &refs[0]),
        2 => (&refs[0], &refs[1], &refs[0]),
        _ => (&refs[0], &refs[1], &refs[2]),
    };
    format!(
        "{fm}SELECT\n    a.{c1},\n    b.{c2},\n    c.{c3},\n    c.{c4}\nFROM {r1} a\nINNER JOIN {r2} b ON a.user_id = b.user_id\nLEFT JOIN {r3} c ON a.user_id = c.user_id\n",
        fm = frontmatter(),
        c1 = cols[0],
        c2 = cols[1],
        c3 = cols[2],
        c4 = cols[3],
    )
}

/// 5. Window function (ROW_NUMBER/RANK/LAG).
fn window_function(rng: &mut ChaChaRng, refs: &[String]) -> String {
    let cols = pick_columns(rng, 2);
    let win_fn = match rng.gen_range(0..3) {
        0 => "ROW_NUMBER()",
        1 => "RANK()",
        _ => "LAG(amount, 1)",
    };
    let source = &refs[0];
    format!(
        "{fm}SELECT\n    {c1},\n    {c2},\n    {wf} OVER (PARTITION BY {c1} ORDER BY created_at) AS win_val\nFROM {source}\n",
        fm = frontmatter(),
        c1 = cols[0],
        c2 = cols[1],
        wf = win_fn,
    )
}

/// 6. CTE pattern with 1-2 CTEs.
fn cte_pattern(rng: &mut ChaChaRng, refs: &[String]) -> String {
    let cols = pick_columns(rng, 3);
    let cond = pick_condition(rng);
    let source = &refs[0];
    format!(
        "{fm}WITH filtered AS (\n    SELECT {c1}, {c2}, {c3}\n    FROM {source}\n    WHERE {cond}\n),\naggregated AS (\n    SELECT {c1}, COUNT(*) AS cnt\n    FROM filtered\n    GROUP BY {c1}\n)\nSELECT\n    a.{c1},\n    a.cnt,\n    f.{c2}\nFROM aggregated a\nINNER JOIN filtered f ON a.{c1} = f.{c1}\n",
        fm = frontmatter(),
        c1 = cols[0],
        c2 = cols[1],
        c3 = cols[2],
    )
}

/// 7. UNION ALL of 2-3 SELECTs.
fn union_all(rng: &mut ChaChaRng, refs: &[String]) -> String {
    let cols = pick_columns(rng, 3);
    let mut parts = Vec::new();
    let num_unions = refs.len().clamp(2, 3);
    for i in 0..num_unions {
        let r = &refs[i % refs.len()];
        let tag = format!("'source_{}'", i);
        parts.push(format!(
            "SELECT {c1}, {c2}, {c3}, {tag} AS source_tag FROM {r}",
            c1 = cols[0],
            c2 = cols[1],
            c3 = cols[2],
        ));
    }
    format!("{}{}\n", frontmatter(), parts.join("\nUNION ALL\n"))
}

/// 8. CASE WHEN categorization.
fn case_expression(rng: &mut ChaChaRng, refs: &[String]) -> String {
    let cols = pick_columns(rng, 2);
    let source = &refs[0];
    format!(
        "{fm}SELECT\n    {c1},\n    {c2},\n    CASE\n        WHEN amount > 1000 THEN 'high'\n        WHEN amount > 100 THEN 'medium'\n        ELSE 'low'\n    END AS value_tier\nFROM {source}\n",
        fm = frontmatter(),
        c1 = cols[0],
        c2 = cols[1],
    )
}

/// 9. WHERE col IN (SELECT ...) subquery filter.
fn subquery_filter(rng: &mut ChaChaRng, refs: &[String]) -> String {
    let cols = pick_columns(rng, 3);
    let (r1, r2) = if refs.len() >= 2 {
        (&refs[0], &refs[1])
    } else {
        (&refs[0], &refs[0])
    };
    format!(
        "{fm}SELECT\n    {c1},\n    {c2},\n    {c3}\nFROM {r1}\nWHERE user_id IN (\n    SELECT user_id FROM {r2} WHERE {cond}\n)\n",
        fm = frontmatter(),
        c1 = cols[0],
        c2 = cols[1],
        c3 = cols[2],
        cond = pick_condition(rng),
    )
}

/// 10. DATE_TRUNC + GROUP BY for time-series.
fn date_aggregation(rng: &mut ChaChaRng, refs: &[String]) -> String {
    let trunc = match rng.gen_range(0..3) {
        0 => "day",
        1 => "week",
        _ => "month",
    };
    let aggs = pick_aggs(rng, 2);
    let source = &refs[0];
    format!(
        "{fm}SELECT\n    DATE_TRUNC('{trunc}', event_time) AS period,\n    {a1} AS metric_1,\n    {a2} AS metric_2\nFROM {source}\nGROUP BY DATE_TRUNC('{trunc}', event_time)\n",
        fm = frontmatter(),
        a1 = aggs[0],
        a2 = aggs[1],
    )
}

/// 11. GROUP BY with HAVING clause.
fn having_filter(rng: &mut ChaChaRng, refs: &[String]) -> String {
    let group_col = pick_columns(rng, 1)[0];
    let aggs = pick_aggs(rng, 2);
    let source = &refs[0];
    format!(
        "{fm}SELECT\n    {group_col},\n    {a1} AS val_1,\n    {a2} AS val_2\nFROM {source}\nGROUP BY {group_col}\nHAVING COUNT(*) > 10\n",
        fm = frontmatter(),
        a1 = aggs[0],
        a2 = aggs[1],
    )
}

/// 12. Complex mixed: CTE + JOIN + aggregation.
fn complex_mixed(rng: &mut ChaChaRng, refs: &[String]) -> String {
    let cols = pick_columns(rng, 3);
    let agg = pick_aggs(rng, 1)[0];
    let cond = pick_condition(rng);
    let (r1, r2) = if refs.len() >= 2 {
        (&refs[0], &refs[1])
    } else {
        (&refs[0], &refs[0])
    };
    format!(
        "{fm}WITH base AS (\n    SELECT {c1}, {c2}, {c3}\n    FROM {r1}\n    WHERE {cond}\n)\nSELECT\n    b.{c1},\n    {agg} AS agg_val\nFROM base b\nINNER JOIN {r2} j ON b.user_id = j.user_id\nGROUP BY b.{c1}\n",
        fm = frontmatter(),
        c1 = cols[0],
        c2 = cols[1],
        c3 = cols[2],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_gen::graph_builder::{ModelSpec, ModelType};

    #[test]
    fn test_all_templates_produce_valid_sql() {
        let mut rng = ChaChaRng::seed_from_u64(42);

        for i in 0..12 {
            let spec = ModelSpec {
                name: format!("test_model_{}", i),
                layer: 2,
                dependencies: vec!["dep_a".into(), "dep_b".into(), "dep_c".into()],
                model_type: ModelType::Sql,
            };

            let sql = generate_sql(&mut rng, &spec);

            // Strip frontmatter for parsing
            let clean = smelt_parser::strip_frontmatter(&sql);
            let parse = smelt_parser::parse(&clean);

            assert!(
                parse.errors.is_empty(),
                "Template {} produced parse errors: {:?}\nSQL:\n{}",
                i,
                parse.errors,
                sql,
            );
        }
    }

    #[test]
    fn test_frontmatter_present() {
        let mut rng = ChaChaRng::seed_from_u64(42);
        let spec = ModelSpec {
            name: "test".into(),
            layer: 1,
            dependencies: vec!["source".into()],
            model_type: ModelType::Sql,
        };

        let sql = generate_sql(&mut rng, &spec);
        assert!(sql.starts_with("---\n"));
        assert!(sql.contains("materialization: table"));
        assert!(sql.contains("partition_column: event_date"));
    }
}
