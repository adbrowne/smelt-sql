use crate::analysis::{analyze_select, SelectItemKind};
use crate::graph::ModelInfo;
use crate::types::{Opportunity, OpportunityData, Transformation};

/// Detect incremental materialization opportunity from frontmatter config.
pub fn detect(model: &ModelInfo) -> Result<Option<Opportunity>, String> {
    let inc_config = match &model.incremental_config {
        Some(c) => c,
        None => return Ok(None),
    };

    let analysis = analyze_select(&model.sql).ok_or_else(|| {
        format!(
            "Model '{}': has incremental config but SQL could not be parsed",
            model.name
        )
    })?;

    let partition_col = &inc_config.partition_column;

    // Validate partition_column alias exists in SELECT list
    let partition_item = analysis.items.iter().find(|item| match item {
        SelectItemKind::GroupByKey { alias, .. } => alias == partition_col,
        SelectItemKind::CountDistinct { alias, .. } => alias == partition_col,
        SelectItemKind::OtherAggregate { alias, .. } => alias == partition_col,
    });

    let partition_item = partition_item.ok_or_else(|| {
        format!(
            "Model '{}': incremental partition_column '{}' not found as alias in SELECT list",
            model.name, partition_col
        )
    })?;

    // Get the expression for the partition column
    let partition_expr = match partition_item {
        SelectItemKind::GroupByKey { text, .. } => text.clone(),
        SelectItemKind::CountDistinct { argument, .. } => argument.clone(),
        SelectItemKind::OtherAggregate { text, .. } => text.clone(),
    };

    // Validate it appears in GROUP BY
    let in_group_by = analysis
        .group_by_exprs
        .iter()
        .any(|expr| expr == &partition_expr);
    if !in_group_by {
        return Err(format!(
            "Model '{}': partition_column '{}' (expression: {}) not found in GROUP BY clause",
            model.name, partition_col, partition_expr
        ));
    }

    // Extract the source time column from the expression
    // e.g., date_trunc('day', event_time) -> event_time
    let event_time_column = extract_time_column(&partition_expr).unwrap_or(partition_expr.clone());

    Ok(Some(Opportunity {
        rule_name: "incremental".to_string(),
        model: model.name.clone(),
        description: format!(
            "Incremental materialization on partition column '{}' (source: '{}')",
            partition_col, event_time_column,
        ),
        data: OpportunityData::Incremental {
            event_time_column: event_time_column.clone(),
            partition_column: partition_col.clone(),
        },
    }))
}

/// Extract the time column argument from common time-truncation expressions.
///
/// Handles patterns like:
/// - `date_trunc('day', event_time)` → `event_time`
/// - `DATE(event_time)` → `event_time`
/// - `event_time` → `event_time` (identity)
fn extract_time_column(expr: &str) -> Option<String> {
    let trimmed = expr.trim();

    // date_trunc('interval', column)
    if let Some(rest) = trimmed
        .strip_prefix("date_trunc(")
        .or_else(|| trimmed.strip_prefix("DATE_TRUNC("))
    {
        let rest = rest.strip_suffix(')')?;
        // Skip the first argument (the interval string)
        let comma_pos = rest.find(',')?;
        let col = rest[comma_pos + 1..].trim();
        return Some(col.to_string());
    }

    // DATE(column)
    if let Some(rest) = trimmed
        .strip_prefix("DATE(")
        .or_else(|| trimmed.strip_prefix("date("))
    {
        let col = rest.strip_suffix(')')?.trim();
        return Some(col.to_string());
    }

    // Simple column reference (no parens)
    if !trimmed.contains('(') {
        return Some(trimmed.to_string());
    }

    None
}

/// Produce a SetIncremental transformation for a model.
pub fn optimize(model: &ModelInfo) -> Result<Option<Transformation>, String> {
    let opportunity = detect(model)?;
    match opportunity {
        None => Ok(None),
        Some(opp) => match &opp.data {
            OpportunityData::Incremental {
                event_time_column,
                partition_column,
            } => Ok(Some(Transformation::SetIncremental {
                model: model.name.clone(),
                event_time_column: event_time_column.clone(),
                partition_column: partition_column.clone(),
            })),
            _ => Ok(None),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::IncrementalConfig;

    fn model(name: &str, sql: &str, partition_column: &str) -> ModelInfo {
        ModelInfo {
            name: name.to_string(),
            sql: sql.to_string(),
            refs: vec![],
            incremental_config: Some(IncrementalConfig {
                partition_column: partition_column.to_string(),
            }),
        }
    }

    #[test]
    fn test_detect_incremental() {
        let m = model(
            "daily",
            "SELECT date_trunc('day', event_time) as event_date, user_id, COUNT(*) as cnt FROM events GROUP BY 1, 2",
            "event_date",
        );
        let opp = detect(&m).unwrap().unwrap();
        assert_eq!(opp.rule_name, "incremental");
        match opp.data {
            OpportunityData::Incremental {
                ref event_time_column,
                ref partition_column,
            } => {
                assert_eq!(event_time_column, "event_time");
                assert_eq!(partition_column, "event_date");
            }
            _ => panic!("Expected Incremental data"),
        }
    }

    #[test]
    fn test_detect_no_config() {
        let m = ModelInfo {
            name: "test".to_string(),
            sql: "SELECT a FROM t GROUP BY 1".to_string(),
            refs: vec![],
            incremental_config: None,
        };
        assert!(detect(&m).unwrap().is_none());
    }

    #[test]
    fn test_detect_invalid_partition_column() {
        let m = model("test", "SELECT a FROM t GROUP BY 1", "nonexistent_column");
        let result = detect(&m);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found as alias"));
    }

    #[test]
    fn test_extract_time_column_date_trunc() {
        assert_eq!(
            extract_time_column("date_trunc('day', event_time)"),
            Some("event_time".to_string())
        );
    }

    #[test]
    fn test_extract_time_column_date_func() {
        assert_eq!(
            extract_time_column("DATE(event_time)"),
            Some("event_time".to_string())
        );
    }

    #[test]
    fn test_extract_time_column_simple() {
        assert_eq!(
            extract_time_column("event_date"),
            Some("event_date".to_string())
        );
    }

    #[test]
    fn test_optimize_produces_transformation() {
        let m = model(
            "daily",
            "SELECT date_trunc('day', event_time) as event_date, user_id, COUNT(*) as cnt FROM events GROUP BY 1, 2",
            "event_date",
        );
        let t = optimize(&m).unwrap().unwrap();
        match t {
            Transformation::SetIncremental {
                model,
                event_time_column,
                partition_column,
            } => {
                assert_eq!(model, "daily");
                assert_eq!(event_time_column, "event_time");
                assert_eq!(partition_column, "event_date");
            }
            _ => panic!("Expected SetIncremental"),
        }
    }
}
