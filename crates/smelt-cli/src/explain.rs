use crate::discovery::ModelFile;
use crate::logical_graph::LogicalGraph;
use crate::physical_graph::{PhysicalGraph, PhysicalStrategy};
use anyhow::Result;
use serde::Serialize;
use smelt_core::{Granularity, IncrementalConfig, Materialization};
use smelt_planner::{analyze_batch_safety, BatchSafety, ModelInfo};
use std::collections::BTreeMap;

/// Top-level JSON output for `smelt explain --json`.
#[derive(Debug, Serialize)]
pub struct ExplainOutput {
    pub models: BTreeMap<String, ExplainModel>,
    pub execution_order: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physical: Option<ExplainPhysical>,
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

/// Physical execution plan section of explain output.
#[derive(Debug, Serialize)]
pub struct ExplainPhysical {
    pub execution_order: Vec<String>,
    pub nodes: BTreeMap<String, ExplainPhysicalNode>,
    pub ephemerals: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub transformations: Vec<String>,
}

/// Per-node metadata in the physical explain output.
#[derive(Debug, Serialize)]
pub struct ExplainPhysicalNode {
    pub strategy: String,
    pub materialization: Materialization,
    pub target: String,
    #[serde(skip_serializing_if = "is_self_origin")]
    pub logical_origins: Vec<String>,
}

fn is_self_origin(origins: &[String]) -> bool {
    origins.len() == 1
}

/// Build the explain output from the logical graph (config already resolved on nodes).
pub fn build_explain_output(graph: &LogicalGraph) -> Result<ExplainOutput> {
    let execution_order = graph.execution_order()?;

    let mut models = BTreeMap::new();
    for node in graph.iter_nodes() {
        let model_file = &node.model_file;
        let metadata = model_file.metadata.as_deref();

        let incremental = node.incremental.as_ref().map(|inc| {
            let batch_safety = compute_batch_safety_label(&node.name, model_file, inc);
            ExplainIncremental {
                granularity: inc.granularity.clone(),
                partition_column: inc.partition_column.clone(),
                event_time_column: inc.event_time_column.clone(),
                unique_key: inc.unique_key.clone(),
                batch_safety,
            }
        });

        let owner = metadata.and_then(|m| m.owner.clone());
        let dependencies = graph.get_upstream(&node.name);

        models.insert(
            node.name.clone(),
            ExplainModel {
                dependencies,
                materialization: node.materialization.clone(),
                incremental,
                tags: node.tags.clone(),
                owner,
            },
        );
    }

    Ok(ExplainOutput {
        models,
        execution_order,
        physical: None,
    })
}

/// Build physical explain section from a PhysicalGraph and the logical graph (for ephemeral list).
pub fn build_physical_explain(physical: &PhysicalGraph, logical: &LogicalGraph) -> ExplainPhysical {
    let mut nodes = BTreeMap::new();
    for node in physical.iter_in_order() {
        let strategy = match &node.strategy {
            PhysicalStrategy::FullRefresh => "full_refresh".to_string(),
            PhysicalStrategy::CubeSplit { steps } => {
                format!("cube_split ({} steps)", steps.len())
            }
            PhysicalStrategy::Incremental {
                config, plan_steps, ..
            } => {
                let base = format!(
                    "incremental (partition: {}, granularity: {})",
                    config.partition_column,
                    serde_json::to_value(&config.granularity)
                        .ok()
                        .and_then(|v| v.as_str().map(|s| s.to_string()))
                        .unwrap_or_else(|| "?".to_string()),
                );
                if plan_steps.is_some() {
                    format!("{} + plan steps", base)
                } else {
                    base
                }
            }
        };

        nodes.insert(
            node.name.clone(),
            ExplainPhysicalNode {
                strategy,
                materialization: node.materialization.clone(),
                target: node.target.clone(),
                logical_origins: node.logical_origins.clone(),
            },
        );
    }

    // Collect ephemeral models from logical graph
    let ephemerals: Vec<String> = logical
        .iter_nodes()
        .filter(|n| n.materialization == Materialization::Ephemeral)
        .map(|n| n.name.clone())
        .collect();

    // Planner summary as transformation descriptions
    let transformations: Vec<String> = physical
        .planner_summary()
        .into_iter()
        .map(|(model, desc)| format!("{} → {}", model, desc))
        .collect();

    ExplainPhysical {
        execution_order: physical.execution_order().to_vec(),
        nodes,
        ephemerals,
        transformations,
    }
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
    use crate::config::{Config, ModelConfig, Target};
    use crate::discovery::ModelKind;
    use rowan::TextRange;
    use smelt_core::RefInfo;
    use std::collections::HashMap;

    fn make_model(name: &str, deps: Vec<&str>, content: &str) -> crate::discovery::ModelFile {
        let refs = deps
            .into_iter()
            .map(|dep| RefInfo {
                model_name: dep.to_string(),
                has_named_params: false,
                range: TextRange::default(),
                smelt_ref: smelt_core::refs::SmeltRef::LegacyRef(dep.to_string()),
            })
            .collect();

        let path: std::path::PathBuf = format!("{}.sql", name).into();
        crate::discovery::ModelFile {
            name: name.to_string(),
            model_id: smelt_core::ModelId::from_path(path.clone()),
            path,
            content: content.to_string(),
            refs,
            parse_errors: Vec::new(),
            metadata: None,
            kind: ModelKind::Sql,
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
                warehouse: None,
                format: None,
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
        let config = make_config(vec![]);
        let graph = LogicalGraph::build(models, None, &[], &config, "dev").unwrap();

        let output = build_explain_output(&graph).unwrap();

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
                target: None,
            },
        )]);
        let graph = LogicalGraph::build(models, None, &[], &config, "dev").unwrap();

        let output = build_explain_output(&graph).unwrap();

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
        let config = make_config(vec![]);
        let graph = LogicalGraph::build(models, None, &[], &config, "dev").unwrap();

        let output = build_explain_output(&graph).unwrap();
        let json = serde_json::to_string_pretty(&output).unwrap();

        assert!(json.contains("\"models\""));
        assert!(json.contains("\"execution_order\""));
        assert!(json.contains("\"a\""));
    }

    #[test]
    fn test_physical_explain_basic() {
        use crate::physical_graph::PhysicalGraphBuilder;

        let models = vec![
            make_model("orders", vec![], "SELECT * FROM raw_orders"),
            make_model(
                "daily_revenue",
                vec!["orders"],
                "SELECT date, SUM(amount) FROM smelt.ref('orders') GROUP BY date",
            ),
        ];
        let config = make_config(vec![]);
        let graph = LogicalGraph::build(models, None, &[], &config, "dev").unwrap();

        let target_schemas: HashMap<String, String> = config
            .targets
            .iter()
            .map(|(k, v)| (k.clone(), v.schema.clone()))
            .collect();
        let pg = PhysicalGraphBuilder::for_explain(&graph, &[], target_schemas)
            .build()
            .unwrap();

        let phys = build_physical_explain(&pg, &graph);

        assert_eq!(phys.execution_order.len(), 2);
        assert_eq!(phys.execution_order[0], "orders");
        assert_eq!(phys.execution_order[1], "daily_revenue");
        assert!(phys.ephemerals.is_empty());
        assert!(phys.transformations.is_empty());

        let orders_node = &phys.nodes["orders"];
        assert_eq!(orders_node.strategy, "full_refresh");
        assert_eq!(orders_node.target, "dev");
    }

    #[test]
    fn test_physical_explain_with_ephemeral() {
        use crate::physical_graph::PhysicalGraphBuilder;

        let models = vec![
            make_model("staging", vec![], "SELECT * FROM raw"),
            make_model(
                "mart",
                vec!["staging"],
                "SELECT * FROM smelt.ref('staging')",
            ),
        ];
        let mut config = make_config(vec![(
            "staging",
            ModelConfig {
                materialization: Some(Materialization::Ephemeral),
                incremental: None,
                tags: vec![],
                target: None,
            },
        )]);
        // Need mart to be non-ephemeral
        config
            .models
            .entry("mart".to_string())
            .or_insert(ModelConfig {
                materialization: Some(Materialization::Table),
                incremental: None,
                tags: vec![],
                target: None,
            });

        let graph = LogicalGraph::build(models, None, &[], &config, "dev").unwrap();

        let target_schemas: HashMap<String, String> = config
            .targets
            .iter()
            .map(|(k, v)| (k.clone(), v.schema.clone()))
            .collect();
        let pg = PhysicalGraphBuilder::for_explain(&graph, &[], target_schemas)
            .build()
            .unwrap();

        let phys = build_physical_explain(&pg, &graph);

        // Ephemeral should not be in physical execution order
        assert_eq!(phys.execution_order.len(), 1);
        assert_eq!(phys.execution_order[0], "mart");
        // But should appear in ephemerals list
        assert_eq!(phys.ephemerals, vec!["staging"]);
    }

    #[test]
    fn test_physical_explain_json_includes_physical() {
        use crate::physical_graph::PhysicalGraphBuilder;

        let models = vec![make_model("a", vec![], "SELECT 1")];
        let config = make_config(vec![]);
        let graph = LogicalGraph::build(models, None, &[], &config, "dev").unwrap();

        let target_schemas: HashMap<String, String> = config
            .targets
            .iter()
            .map(|(k, v)| (k.clone(), v.schema.clone()))
            .collect();
        let pg = PhysicalGraphBuilder::for_explain(&graph, &[], target_schemas)
            .build()
            .unwrap();

        let mut output = build_explain_output(&graph).unwrap();
        output.physical = Some(build_physical_explain(&pg, &graph));

        let json = serde_json::to_string_pretty(&output).unwrap();
        assert!(json.contains("\"physical\""));
        assert!(json.contains("\"full_refresh\""));
        assert!(json.contains("\"ephemerals\""));
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
        let config = make_config(vec![]);
        let graph = LogicalGraph::build(models, None, &[], &config, "dev").unwrap();

        let output = build_explain_output(&graph).unwrap();
        assert_eq!(
            output.models["orders"].owner.as_deref(),
            Some("analytics-team")
        );
    }
}
