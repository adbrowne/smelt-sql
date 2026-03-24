use crate::config::Config;
use crate::discovery::ModelFile;
use crate::graph::DependencyGraph;
use anyhow::Result;
use serde::Serialize;
use smelt_core::{Granularity, IncrementalConfig, Materialization};
use smelt_optimizer::{analyze_batch_safety, BatchSafety, ModelInfo};
use std::collections::BTreeMap;

/// Top-level JSON output for `smelt explain --json`.
#[derive(Debug, Serialize)]
pub struct ExplainOutput {
    pub models: BTreeMap<String, ExplainModel>,
    pub execution_order: Vec<String>,
}

/// Per-model metadata in the explain output.
#[derive(Debug, Serialize)]
pub struct ExplainModel {
    pub dependencies: Vec<String>,
    pub materialization: Materialization,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incremental: Option<ExplainIncremental>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
}

/// Incremental-specific metadata in the explain output.
#[derive(Debug, Serialize)]
pub struct ExplainIncremental {
    pub granularity: Granularity,
    pub partition_column: String,
    pub event_time_column: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unique_key: Vec<String>,
    pub batch_safety: String,
}

/// Build the explain output from the dependency graph and config.
pub fn build_explain_output(graph: &DependencyGraph, config: &Config) -> Result<ExplainOutput> {
    let execution_order = graph.execution_order()?;

    let mut models = BTreeMap::new();
    for (name, model_file) in graph.models() {
        let metadata = model_file.metadata.as_deref();

        let materialization = config.get_materialization_with_metadata(name, metadata);

        let incremental_config = config.get_incremental_with_metadata(name, metadata);

        let incremental = incremental_config.map(|inc| {
            let batch_safety = compute_batch_safety_label(name, model_file, inc);
            ExplainIncremental {
                granularity: inc.granularity.clone(),
                partition_column: inc.partition_column.clone(),
                event_time_column: inc.event_time_column.clone(),
                unique_key: inc.unique_key.clone(),
                batch_safety,
            }
        });

        let tags = config.get_tags(name, metadata);
        let owner = metadata.and_then(|m| m.owner.clone());

        // Get dependencies, filtering out external sources
        let dependencies = graph.get_upstream(name);

        models.insert(
            name.clone(),
            ExplainModel {
                dependencies,
                materialization,
                incremental,
                tags,
                owner,
            },
        );
    }

    Ok(ExplainOutput {
        models,
        execution_order,
    })
}

fn compute_batch_safety_label(
    name: &str,
    model_file: &ModelFile,
    inc: &IncrementalConfig,
) -> String {
    let model_info = ModelInfo {
        name: name.to_string(),
        sql: model_file.content.clone(),
        refs: model_file
            .refs
            .iter()
            .map(|r| r.model_name.clone())
            .collect(),
        incremental_config: Some(inc.clone()),
    };
    match analyze_batch_safety(&model_info) {
        BatchSafety::FullyBatchSafe => "fully_batch_safe".to_string(),
        BatchSafety::BoundedSafe {
            max_chunk_days,
            context_days,
            ..
        } => format!(
            "bounded_safe(chunk={}d,context={}d)",
            max_chunk_days, context_days
        ),
        BatchSafety::PerPartitionOnly { .. } => "per_partition_only".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ModelConfig, Target};
    use rowan::TextRange;
    use smelt_core::RefInfo;
    use std::collections::HashMap;

    fn make_model(name: &str, deps: Vec<&str>, content: &str) -> ModelFile {
        let refs = deps
            .into_iter()
            .map(|dep| RefInfo {
                model_name: dep.to_string(),
                has_named_params: false,
                range: TextRange::default(),
            })
            .collect();

        ModelFile {
            name: name.to_string(),
            path: format!("{}.sql", name).into(),
            content: content.to_string(),
            refs,
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
        }
    }

    fn make_config(model_configs: Vec<(&str, ModelConfig)>) -> Config {
        let mut models = HashMap::new();
        for (name, mc) in model_configs {
            models.insert(name.to_string(), mc);
        }

        let mut targets = HashMap::new();
        targets.insert(
            "dev".to_string(),
            Target {
                target_type: "duckdb".to_string(),
                database: Some("test.duckdb".to_string()),
                schema: "main".to_string(),
                connect_url: None,
                catalog: None,
            },
        );

        Config {
            name: "test".to_string(),
            version: 1,
            model_paths: vec!["models".to_string()],
            seed_paths: vec!["seeds".to_string()],
            targets,
            default_materialization: Materialization::View,
            models,
            python: None,
        }
    }

    #[test]
    fn test_explain_basic() {
        let models = vec![
            make_model("orders", vec![], "SELECT * FROM raw_orders"),
            make_model(
                "daily_revenue",
                vec!["orders"],
                "SELECT date, SUM(amount) FROM smelt.ref('orders') GROUP BY date",
            ),
        ];
        let graph = DependencyGraph::build(models, None).unwrap();
        let config = make_config(vec![]);

        let output = build_explain_output(&graph, &config).unwrap();

        assert_eq!(output.execution_order.len(), 2);
        assert_eq!(output.execution_order[0], "orders");
        assert_eq!(output.execution_order[1], "daily_revenue");
        assert_eq!(output.models.len(), 2);

        let orders = &output.models["orders"];
        assert!(orders.dependencies.is_empty());
        assert_eq!(orders.materialization, Materialization::View);
        assert!(orders.incremental.is_none());

        let daily = &output.models["daily_revenue"];
        assert_eq!(daily.dependencies, vec!["orders"]);
    }

    #[test]
    fn test_explain_with_incremental() {
        use smelt_core::{Granularity, IncrementalConfig};

        let models = vec![
            make_model("orders", vec![], "SELECT * FROM raw_orders"),
            make_model(
                "daily_revenue",
                vec!["orders"],
                "SELECT date, SUM(amount) FROM smelt.ref('orders') GROUP BY date",
            ),
        ];
        let graph = DependencyGraph::build(models, None).unwrap();
        let config = make_config(vec![(
            "daily_revenue",
            ModelConfig {
                materialization: Some(Materialization::Table),
                incremental: Some(IncrementalConfig {
                    enabled: true,
                    event_time_column: "created_at".to_string(),
                    partition_column: "order_date".to_string(),
                    granularity: Granularity::Day,
                    unique_key: vec![],
                    safety_overrides: Default::default(),
                }),
                tags: vec!["revenue".to_string(), "daily".to_string()],
            },
        )]);

        let output = build_explain_output(&graph, &config).unwrap();

        let daily = &output.models["daily_revenue"];
        assert_eq!(daily.materialization, Materialization::Table);
        assert_eq!(daily.tags, vec!["revenue", "daily"]);

        let inc = daily.incremental.as_ref().unwrap();
        assert_eq!(inc.partition_column, "order_date");
        assert_eq!(inc.event_time_column, "created_at");
        assert_eq!(inc.batch_safety, "fully_batch_safe");
    }

    #[test]
    fn test_explain_json_serialization() {
        let models = vec![make_model("a", vec![], "SELECT 1")];
        let graph = DependencyGraph::build(models, None).unwrap();
        let config = make_config(vec![]);

        let output = build_explain_output(&graph, &config).unwrap();
        let json = serde_json::to_string_pretty(&output).unwrap();

        assert!(json.contains("\"models\""));
        assert!(json.contains("\"execution_order\""));
        assert!(json.contains("\"a\""));
    }

    #[test]
    fn test_explain_with_owner_from_metadata() {
        use crate::metadata::ModelMetadata;

        let mut model = make_model("orders", vec![], "SELECT 1");
        model.metadata = Some(Box::new(ModelMetadata {
            owner: Some("analytics-team".to_string()),
            ..Default::default()
        }));

        let models = vec![model];
        let graph = DependencyGraph::build(models, None).unwrap();
        let config = make_config(vec![]);

        let output = build_explain_output(&graph, &config).unwrap();
        assert_eq!(
            output.models["orders"].owner.as_deref(),
            Some("analytics-team")
        );
    }
}
