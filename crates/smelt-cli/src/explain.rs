use crate::discovery::ModelFile;
use crate::logical_graph::{LogicalGraph, LogicalNode};
use crate::physical_graph::{PhysicalGraph, PhysicalStrategy};
use anyhow::Result;
use serde::Serialize;
use smelt_core::config::TimeseriesConfig;
use smelt_core::{Granularity, IncrementalConfig, Materialization, ModelOriginKind};
use smelt_planner::{analyze_batch_safety, BatchSafety, BoundContext, BoundResult, ModelInfo};
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
    /// For generator-emitted models: provenance identifying the generator file
    /// and the `ModelDef.name` that produced this model. Omitted for hand-authored
    /// models (per `docs/specs/cli.md` §"`smelt explain --json` output schema").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<ModelOriginKind>,
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
    /// Per-source bound map derived from the model's SQL.
    /// Maps source name → bound result. Only timeseries sources appear;
    /// lookup sources (no `timeseries:`) are absent.
    /// Omitted when the model has no timeseries upstream refs.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub source_bounds: BTreeMap<String, SourceBoundJson>,
}

/// JSON shape for one source's bound in `smelt explain --json`.
#[derive(Debug, Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SourceBoundJson {
    Bounded {
        partition_col: String,
        /// ISO-8601 duration (e.g. "P1D", "PT30M", "PT0S").
        before: String,
        /// ISO-8601 duration.
        after: String,
    },
    Unbounded,
    NotDerivable,
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
pub fn build_explain_output(
    graph: &LogicalGraph,
    fn_bodies: &smelt_runtime::FnBodyMap,
) -> Result<ExplainOutput> {
    let execution_order = graph.execution_order()?;

    let mut models = BTreeMap::new();
    for node in graph.iter_nodes() {
        let model_file = &node.model_file;
        let metadata = model_file.metadata.as_deref();

        let incremental = match (&node.incremental, &node.timeseries) {
            (Some(inc), Some(ts)) => {
                // Classify and derive bounds on the *expanded* SQL so a
                // `RANGE BETWEEN INTERVAL` (or any Form A/B pattern) declared
                // inside a `smelt.define` body is seen — matching the execution
                // path. The planner is pure (no function registry), so the
                // expansion happens here in the CLI layer.
                let expanded_sql =
                    smelt_runtime::expand_function_calls(&model_file.content, fn_bodies);
                let batch_safety =
                    compute_batch_safety_label(&node.name, &expanded_sql, model_file, inc, ts);
                let source_bounds = compute_source_bounds_for_node(node, &expanded_sql, graph);
                Some(ExplainIncremental {
                    granularity: ts.granularity.clone(),
                    partition_column: ts.partition_column.clone(),
                    event_time_column: ts.event_time_column.clone(),
                    unique_key: inc.unique_key.clone(),
                    batch_safety,
                    source_bounds,
                })
            }
            _ => None,
        };

        let owner = metadata.and_then(|m| m.owner.clone());
        let dependencies = graph.get_upstream(&node.name);

        // Build origin for generator-emitted models.
        let origin = match (&node.generator_file, &node.generator_name) {
            (Some(gf), Some(gn)) => Some(ModelOriginKind::Generated {
                generator_file: gf.clone(),
                generator_name: gn.clone(),
            }),
            _ => None,
        };

        models.insert(
            node.name.clone(),
            ExplainModel {
                dependencies,
                materialization: node.materialization.clone(),
                incremental,
                tags: node.tags.clone(),
                owner,
                origin,
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
                timeseries,
                plan_steps,
                ..
            } => {
                let base = format!(
                    "incremental (partition: {}, granularity: {})",
                    timeseries.partition_column,
                    serde_json::to_value(&timeseries.granularity)
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

/// Derive per-source bounds for a node's incremental model using the logical graph
/// to resolve upstream timeseries configs.
fn compute_source_bounds_for_node(
    node: &LogicalNode,
    sql: &str,
    graph: &LogicalGraph,
) -> BTreeMap<String, SourceBoundJson> {
    use smelt_planner::analysis::source_bounds::derive_model_bounds;
    use smelt_planner::Frontmatter;

    let mut ctx = BoundContext::new();
    for dep_name in &node.dependencies {
        if let Ok(upstream) = graph.get_node(dep_name) {
            if let Some(ts) = &upstream.timeseries {
                ctx.add_source(dep_name, &ts.partition_column);
            }
        }
    }

    let stripped = Frontmatter::strip(sql);
    let raw_bounds = derive_model_bounds(stripped, &ctx);

    let mut result = BTreeMap::new();
    for (source_name, bound) in raw_bounds {
        let json = match bound {
            BoundResult::Bounded {
                source_partition_col,
                before,
                after,
            } => SourceBoundJson::Bounded {
                partition_col: source_partition_col,
                before: before.to_iso8601(),
                after: after.to_iso8601(),
            },
            BoundResult::Unbounded => SourceBoundJson::Unbounded,
            BoundResult::NotDerivable => SourceBoundJson::NotDerivable,
        };
        result.insert(source_name, json);
    }
    result
}

fn compute_batch_safety_label(
    name: &str,
    sql: &str,
    model_file: &ModelFile,
    inc: &IncrementalConfig,
    ts: &TimeseriesConfig,
) -> String {
    let model_info = ModelInfo {
        name: name.to_string(),
        sql: sql.to_string(),
        refs: model_file
            .refs
            .iter()
            .map(|r| r.smelt_ref.to_path().join("."))
            .collect(),
        incremental_config: Some(inc.clone()),
        timeseries_config: Some(ts.clone()),
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
                has_named_params: false,
                range: TextRange::default(),
                smelt_ref: smelt_core::refs::SmeltRef::Path(vec![
                    "models".to_string(),
                    dep.to_string(),
                ]),
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
            address_segments: vec![name.to_string()],
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
            paths: vec!["models".to_string()],
            targets,
            default_materialization: Materialization::View,
            models,
            python: None,
            target: None,
        }
    }

    #[test]
    fn test_batch_safety_uses_expanded_function_body() {
        use smelt_core::config::TimeseriesConfig;
        use smelt_core::{Granularity, IncrementalConfig};

        // A model whose only lookback lives inside a `smelt.define` body must
        // classify as `bounded_safe` — but only when the explain path expands
        // the function. With no registry the outer SQL shows no lookback and it
        // falls back to `fully_batch_safe`. This is the classification-path
        // counterpart to the execution-path expansion.
        let content = "SELECT device_id, d FROM smelt.functions.windowed(src => raw_events)";
        let models = vec![make_model("sessions", vec![], content)];
        let config = make_config(vec![(
            "sessions",
            ModelConfig {
                materialization: Some(Materialization::Table),
                timeseries: Some(TimeseriesConfig {
                    event_time_column: "d".to_string(),
                    partition_column: "d".to_string(),
                    granularity: Granularity::Day,
                    week_start: None,
                }),
                incremental: Some(IncrementalConfig {
                    enabled: true,
                    unique_key: vec![],
                    safety_overrides: Default::default(),
                }),
                tags: vec![],
                target: None,
            },
        )]);
        let graph = LogicalGraph::build(models, None, &[], &config, "dev").unwrap();

        let mut fn_bodies: smelt_runtime::FnBodyMap = HashMap::new();
        fn_bodies.insert(
            "windowed".to_string(),
            (
                vec![("src".to_string(), None)],
                "(SELECT device_id, d, LAG(d) OVER (PARTITION BY device_id ORDER BY d \
                 RANGE BETWEEN INTERVAL '1 day' PRECEDING AND CURRENT ROW) AS p FROM src)"
                    .to_string(),
            ),
        );

        let bs = |fns: &smelt_runtime::FnBodyMap| {
            build_explain_output(&graph, fns).unwrap().models["sessions"]
                .incremental
                .as_ref()
                .unwrap()
                .batch_safety
                .clone()
        };

        let with_registry = bs(&fn_bodies);
        let without_registry = bs(&HashMap::new());

        assert!(
            with_registry.starts_with("bounded_safe"),
            "with the registry the function-internal RANGE is seen: {with_registry}"
        );
        assert_eq!(
            without_registry, "fully_batch_safe",
            "without the registry the outer SQL shows no lookback: {without_registry}"
        );
    }

    #[test]
    fn test_explain_basic() {
        let models = vec![
            make_model("orders", vec![], "SELECT * FROM raw_orders"),
            make_model(
                "daily_revenue",
                vec!["orders"],
                "SELECT date, SUM(amount) FROM smelt.models.orders GROUP BY date",
            ),
        ];
        let config = make_config(vec![]);
        let graph = LogicalGraph::build(models, None, &[], &config, "dev").unwrap();

        let output = build_explain_output(&graph, &HashMap::new()).unwrap();

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
        use smelt_core::config::TimeseriesConfig;
        use smelt_core::{Granularity, IncrementalConfig};

        let models = vec![
            make_model("orders", vec![], "SELECT * FROM raw_orders"),
            make_model(
                "daily_revenue",
                vec!["orders"],
                "SELECT date, SUM(amount) FROM smelt.models.orders GROUP BY date",
            ),
        ];
        let config = make_config(vec![(
            "daily_revenue",
            ModelConfig {
                materialization: Some(Materialization::Table),
                timeseries: Some(TimeseriesConfig {
                    event_time_column: "created_at".to_string(),
                    partition_column: "order_date".to_string(),
                    granularity: Granularity::Day,
                    week_start: None,
                }),
                incremental: Some(IncrementalConfig {
                    enabled: true,
                    unique_key: vec![],
                    safety_overrides: Default::default(),
                }),
                tags: vec!["revenue".to_string(), "daily".to_string()],
                target: None,
            },
        )]);
        let graph = LogicalGraph::build(models, None, &[], &config, "dev").unwrap();

        let output = build_explain_output(&graph, &HashMap::new()).unwrap();

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

        let output = build_explain_output(&graph, &HashMap::new()).unwrap();
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
                "SELECT date, SUM(amount) FROM smelt.models.orders GROUP BY date",
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
                "SELECT * FROM smelt.models.staging",
            ),
        ];
        let mut config = make_config(vec![(
            "staging",
            ModelConfig {
                materialization: Some(Materialization::Ephemeral),
                timeseries: None,
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
                timeseries: None,
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

        let mut output = build_explain_output(&graph, &HashMap::new()).unwrap();
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

        let output = build_explain_output(&graph, &HashMap::new()).unwrap();
        assert_eq!(
            output.models["orders"].owner.as_deref(),
            Some("analytics-team")
        );
    }
}
