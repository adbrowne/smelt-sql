use crate::analysis::{analyze_select, SelectItemKind};
use crate::graph::ModelInfo;
use crate::types::{ExecutionStep, Frontmatter, Opportunity, OpportunityData, Transformation};

/// Detect cube split opportunity from `-- smelt:cube_split` annotation.
///
/// Returns an error string if the annotation is present but the query shape is invalid.
pub fn detect(model: &ModelInfo) -> Result<Option<Opportunity>, String> {
    let sql = Frontmatter::strip(&model.sql);
    let analysis = match analyze_select(sql) {
        Some(a) => a,
        None => return Ok(None),
    };

    if !analysis.has_cube_split_annotation {
        return Ok(None);
    }

    let count_distincts: Vec<_> = analysis
        .items
        .iter()
        .filter(|i| matches!(i, SelectItemKind::CountDistinct { .. }))
        .collect();

    if count_distincts.len() < 2 {
        return Err(format!(
            "Model '{}' has smelt:cube_split annotation but only {} COUNT(DISTINCT ...) \
             expression(s). At least 2 are required.",
            model.name,
            count_distincts.len()
        ));
    }

    let group_by_keys: Vec<String> = analysis
        .items
        .iter()
        .filter_map(|i| match i {
            SelectItemKind::GroupByKey { alias, .. } => Some(alias.clone()),
            _ => None,
        })
        .collect();

    Ok(Some(Opportunity {
        rule_name: "cube_split".to_string(),
        model: model.name.clone(),
        description: format!(
            "Split {} COUNT(DISTINCT) expressions into parallel sub-queries joined on {} GROUP BY key(s)",
            count_distincts.len(),
            group_by_keys.len(),
        ),
        data: OpportunityData::CubeSplit {
            count_distinct_count: count_distincts.len(),
            group_by_keys,
        },
    }))
}

/// Rewrite a model into a cube-split execution plan.
///
/// Strategy: query 0 gets the first COUNT DISTINCT + all non-distinct aggregates.
/// Queries 1..N each get one additional COUNT DISTINCT.
/// A final query joins all temp tables on the GROUP BY keys.
pub fn rewrite(model: &ModelInfo) -> Option<Vec<ExecutionStep>> {
    let sql = Frontmatter::strip(&model.sql);
    let analysis = analyze_select(sql)?;

    // Separate items by kind
    let mut group_keys: Vec<(String, String)> = Vec::new(); // (expr, alias)
    let mut count_distincts: Vec<(String, String)> = Vec::new(); // (argument, alias)
    let mut other_aggs: Vec<(String, String)> = Vec::new(); // (expr, alias)

    for item in &analysis.items {
        match item {
            SelectItemKind::GroupByKey { text, alias, .. } => {
                group_keys.push((text.clone(), alias.clone()));
            }
            SelectItemKind::CountDistinct {
                argument, alias, ..
            } => {
                count_distincts.push((argument.clone(), alias.clone()));
            }
            SelectItemKind::OtherAggregate { text, alias, .. } => {
                other_aggs.push((text.clone(), alias.clone()));
            }
        }
    }

    if count_distincts.len() < 2 {
        return None;
    }

    let from_clause = &analysis.from_text;
    let where_clause = analysis
        .where_text
        .as_ref()
        .map(|w| format!("WHERE {}", w))
        .unwrap_or_default();

    // Build GROUP BY clause from key expressions
    let group_by_clause = if group_keys.is_empty() {
        String::new()
    } else {
        let exprs: Vec<&str> = group_keys.iter().map(|(expr, _)| expr.as_str()).collect();
        format!("GROUP BY {}", exprs.join(", "))
    };

    let mut steps = Vec::new();

    // Query 0: GROUP BY keys + first COUNT DISTINCT + all other aggregates
    {
        let mut select_items: Vec<String> = Vec::new();
        for (expr, alias) in &group_keys {
            select_items.push(format!("{} as {}", expr, alias));
        }
        let (arg, alias) = &count_distincts[0];
        select_items.push(format!("COUNT(DISTINCT {}) as {}", arg, alias));
        for (expr, alias) in &other_aggs {
            select_items.push(format!("{} as {}", expr, alias));
        }

        let sql = format!(
            "SELECT {} {} {} {}",
            select_items.join(", "),
            from_clause,
            where_clause,
            group_by_clause,
        );
        steps.push(ExecutionStep::CreateTemp {
            name: format!("__cube_{}_0", model.name),
            sql: sql.trim().to_string(),
        });
    }

    // Queries 1..N: GROUP BY keys + one COUNT DISTINCT each
    for (i, (arg, alias)) in count_distincts.iter().enumerate().skip(1) {
        let mut select_items: Vec<String> = Vec::new();
        for (expr, key_alias) in &group_keys {
            select_items.push(format!("{} as {}", expr, key_alias));
        }
        select_items.push(format!("COUNT(DISTINCT {}) as {}", arg, alias));

        let sql = format!(
            "SELECT {} {} {} {}",
            select_items.join(", "),
            from_clause,
            where_clause,
            group_by_clause,
        );
        steps.push(ExecutionStep::CreateTemp {
            name: format!("__cube_{}_{}", model.name, i),
            sql: sql.trim().to_string(),
        });
    }

    // Final join query
    let t0 = format!("__cube_{}_0", model.name);
    let mut final_select: Vec<String> = Vec::new();

    // GROUP BY keys from t0
    for (_, alias) in &group_keys {
        final_select.push(format!("t0.{}", alias));
    }
    // First COUNT DISTINCT from t0
    final_select.push(format!("t0.{}", count_distincts[0].1));
    // Other COUNT DISTINCTs from t1, t2, ...
    for (i, (_, alias)) in count_distincts.iter().enumerate().skip(1) {
        final_select.push(format!("t{}.{}", i, alias));
    }
    // Other aggregates from t0
    for (_, alias) in &other_aggs {
        final_select.push(format!("t0.{}", alias));
    }

    // Build JOIN clauses with NULL-safe comparisons
    let mut join_clauses = String::new();
    for i in 1..count_distincts.len() {
        let tn = format!("__cube_{}_{}", model.name, i);
        let conditions: Vec<String> = group_keys
            .iter()
            .map(|(_, alias)| format!("t0.{} IS NOT DISTINCT FROM t{}.{}", alias, i, alias))
            .collect();

        if group_keys.is_empty() {
            // No GROUP BY keys — cross join (single-row results)
            join_clauses.push_str(&format!(" CROSS JOIN {} t{}", tn, i));
        } else {
            join_clauses.push_str(&format!(
                " JOIN {} t{} ON {}",
                tn,
                i,
                conditions.join(" AND ")
            ));
        }
    }

    let final_sql = format!(
        "SELECT {} FROM {} t0{}",
        final_select.join(", "),
        t0,
        join_clauses,
    );
    steps.push(ExecutionStep::FinalQuery {
        sql: final_sql.trim().to_string(),
    });

    // Cleanup temp tables
    for i in 0..count_distincts.len() {
        steps.push(ExecutionStep::DropTemp {
            name: format!("__cube_{}_{}", model.name, i),
        });
    }

    Some(steps)
}

/// Detect and rewrite in one step, producing a Transformation if applicable.
pub fn optimize(model: &ModelInfo) -> Result<Option<Transformation>, String> {
    let opportunity = detect(model)?;
    if opportunity.is_none() {
        return Ok(None);
    }

    let steps = rewrite(model).ok_or_else(|| {
        format!(
            "Model '{}': cube_split annotation detected but rewrite failed",
            model.name
        )
    })?;

    Ok(Some(Transformation::ReplaceWithPlan {
        model: model.name.clone(),
        steps,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(name: &str, sql: &str) -> ModelInfo {
        ModelInfo {
            name: name.to_string(),
            sql: sql.to_string(),
            refs: vec![],
            timeseries_config: None,
            incremental_config: None,
        }
    }

    #[test]
    fn test_detect_with_annotation() {
        let m = model(
            "test",
            "SELECT country, COUNT(DISTINCT user_id) as u, COUNT(DISTINCT session_id) as s FROM events GROUP BY 1 -- smelt:cube_split",
        );
        let opp = detect(&m).unwrap();
        assert!(opp.is_some());
        let opp = opp.unwrap();
        assert_eq!(opp.rule_name, "cube_split");
    }

    #[test]
    fn test_detect_without_annotation() {
        let m = model(
            "test",
            "SELECT country, COUNT(DISTINCT user_id) as u, COUNT(DISTINCT session_id) as s FROM events GROUP BY 1",
        );
        let opp = detect(&m).unwrap();
        assert!(opp.is_none());
    }

    #[test]
    fn test_detect_errors_with_annotation_but_insufficient_count_distinct() {
        let m = model(
            "test",
            "SELECT country, COUNT(DISTINCT user_id) as u FROM events GROUP BY 1 -- smelt:cube_split",
        );
        let result = detect(&m);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("only 1"));
    }

    #[test]
    fn test_rewrite_produces_correct_steps() {
        let m = model(
            "metrics",
            "SELECT country, COUNT(DISTINCT user_id) as unique_users, COUNT(DISTINCT session_id) as unique_sessions FROM events GROUP BY 1 -- smelt:cube_split",
        );
        let steps = rewrite(&m).unwrap();

        // 2 CreateTemp + 1 FinalQuery + 2 DropTemp = 5
        assert_eq!(steps.len(), 5);
        assert!(
            matches!(&steps[0], ExecutionStep::CreateTemp { name, .. } if name == "__cube_metrics_0")
        );
        assert!(
            matches!(&steps[1], ExecutionStep::CreateTemp { name, .. } if name == "__cube_metrics_1")
        );
        assert!(matches!(&steps[2], ExecutionStep::FinalQuery { .. }));
        assert!(
            matches!(&steps[3], ExecutionStep::DropTemp { name, .. } if name == "__cube_metrics_0")
        );
        assert!(
            matches!(&steps[4], ExecutionStep::DropTemp { name, .. } if name == "__cube_metrics_1")
        );
    }

    #[test]
    fn test_rewrite_final_query_joins() {
        let m = model(
            "m",
            "SELECT country, COUNT(DISTINCT user_id) as u, COUNT(DISTINCT session_id) as s FROM events GROUP BY 1 -- smelt:cube_split",
        );
        let steps = rewrite(&m).unwrap();
        let final_sql = match &steps[2] {
            ExecutionStep::FinalQuery { sql } => sql,
            _ => panic!("Expected FinalQuery"),
        };
        assert!(final_sql.contains("IS NOT DISTINCT FROM"));
        assert!(final_sql.contains("t0.country"));
        assert!(final_sql.contains("t0.u"));
        assert!(final_sql.contains("t1.s"));
    }

    #[test]
    fn test_rewrite_preserves_ref_calls() {
        let m = model(
            "m",
            "SELECT country, COUNT(DISTINCT user_id) as u, COUNT(DISTINCT session_id) as s FROM smelt.models.events GROUP BY 1 -- smelt:cube_split",
        );
        let steps = rewrite(&m).unwrap();
        // Both CreateTemp steps should preserve smelt.ref()
        for step in &steps[..2] {
            if let ExecutionStep::CreateTemp { sql, .. } = step {
                assert!(sql.contains("smelt.models.events"));
            }
        }
    }

    #[test]
    fn test_rewrite_with_other_aggregates() {
        let m = model(
            "m",
            "SELECT country, COUNT(DISTINCT user_id) as u, COUNT(DISTINCT session_id) as s, COUNT(*) as total, SUM(revenue) as rev FROM events GROUP BY 1 -- smelt:cube_split",
        );
        let steps = rewrite(&m).unwrap();

        // Query 0 should have other aggregates
        let sql0 = match &steps[0] {
            ExecutionStep::CreateTemp { sql, .. } => sql,
            _ => panic!("Expected CreateTemp"),
        };
        assert!(sql0.contains("COUNT(*)"));
        assert!(sql0.contains("SUM(revenue)"));

        // Query 1 should NOT have other aggregates
        let sql1 = match &steps[1] {
            ExecutionStep::CreateTemp { sql, .. } => sql,
            _ => panic!("Expected CreateTemp"),
        };
        assert!(!sql1.contains("COUNT(*)"));
        assert!(!sql1.contains("SUM(revenue)"));

        // Final should select other aggs from t0
        let final_sql = match &steps[2] {
            ExecutionStep::FinalQuery { sql } => sql,
            _ => panic!("Expected FinalQuery"),
        };
        assert!(final_sql.contains("t0.total"));
        assert!(final_sql.contains("t0.rev"));
    }

    #[test]
    fn test_generated_sql_parses_without_errors() {
        let m = model(
            "m",
            "SELECT country, city, COUNT(DISTINCT user_id) as u, COUNT(DISTINCT session_id) as s FROM events GROUP BY 1, 2 -- smelt:cube_split",
        );
        let steps = rewrite(&m).unwrap();
        for step in &steps {
            let sql = match step {
                ExecutionStep::CreateTemp { sql, .. } => Some(sql),
                ExecutionStep::FinalQuery { sql } => Some(sql),
                _ => None,
            };
            if let Some(sql) = sql {
                let parse = smelt_parser::parse(sql);
                assert!(
                    parse.errors.is_empty(),
                    "Generated SQL has parse errors: {:?}\nSQL: {}",
                    parse.errors,
                    sql
                );
            }
        }
    }
}
