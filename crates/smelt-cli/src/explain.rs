use crate::discovery::ModelFile;
use anyhow::Result;
use serde::Serialize;
use smelt_core::config::{Config, RefreshStrategy, TimeseriesConfig};
use smelt_core::graph::DependencyGraph;
use smelt_core::{BatchedConfig, Granularity, Materialization, ModelOriginKind};
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
    /// Refresh axis: `"cumulative"` when the model uses the cumulative-aggregate
    /// merge loop (`materialization: table` + `refresh: cumulative`). Omitted
    /// when the model uses the default full-refresh strategy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh: Option<RefreshStrategy>,
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

/// Build the explain output from the dependency graph and config.
///
/// `origins` maps emitted model names to `(generator_file, generator_def_name)`.
pub fn build_explain_output(
    graph: &DependencyGraph,
    config: &Config,
    fn_bodies: &smelt_runtime::FnBodyMap,
    origins: &std::collections::HashMap<String, (String, String)>,
) -> Result<ExplainOutput> {
    let execution_order = graph.execution_order()?;

    let mut models = BTreeMap::new();
    for model_name in &execution_order {
        let model_file = graph.get_model(model_name)?;
        let metadata = model_file.metadata.as_deref();
        let frontmatter = smelt_planner::Frontmatter::parse(&model_file.content);

        let materialization = config.get_materialization_with_metadata(model_name, metadata);
        let inc_config = config
            .get_incremental_with_metadata(model_name, metadata)
            .or_else(|| frontmatter.as_ref().and_then(|f| f.batched_config()));
        let ts_config = config
            .get_timeseries_with_metadata(model_name, metadata)
            .cloned()
            .or_else(|| metadata.and_then(|m| m.timeseries.clone()));
        let tags = config.get_tags(model_name, metadata);

        let incremental = match (inc_config, ts_config) {
            (Some(inc), Some(ts)) => {
                // Classify on the *expanded* SQL so a RANGE BETWEEN INTERVAL
                // declared inside a `smelt.define` body is seen.
                let expanded_sql =
                    smelt_runtime::expand_function_calls(&model_file.content, fn_bodies);
                let batch_safety =
                    compute_batch_safety_label(model_name, &expanded_sql, model_file, &inc, &ts);
                let source_bounds = compute_source_bounds(model_name, &expanded_sql, graph, config);
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
        let dependencies = graph.get_upstream(model_name);

        // Build origin for generator-emitted models.
        let origin = origins
            .get(model_name)
            .map(|(gf, gn)| ModelOriginKind::Generated {
                generator_file: gf.clone(),
                generator_name: gn.clone(),
            });

        // Emit `refresh: "cumulative"` when the model is cumulative; omit otherwise.
        let refresh = metadata
            .and_then(|m| m.refresh.clone())
            .filter(|r| *r == RefreshStrategy::Cumulative);

        models.insert(
            model_name.clone(),
            ExplainModel {
                dependencies,
                materialization,
                refresh,
                incremental,
                tags,
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

/// Build the physical explain section from the plan summary and graph.
///
/// The physical section lists per-model strategy (from the `PlanSummary`),
/// ephemerals (from the graph), and planner transformations.
pub fn build_physical_explain(
    plan_summary: &smelt_runtime::PlanSummary,
    graph: &DependencyGraph,
    config: &Config,
    target: &str,
) -> ExplainPhysical {
    let mut nodes = BTreeMap::new();
    let mut ephemerals = Vec::new();

    for record in &plan_summary.models {
        let model_name = &record.name;

        // Collect ephemerals
        if matches!(record.strategy, smelt_runtime::ModelStrategy::Ephemeral) {
            ephemerals.push(model_name.clone());
            continue;
        }

        let strategy = match &record.strategy {
            smelt_runtime::ModelStrategy::FullRefresh => "full_refresh".to_string(),
            smelt_runtime::ModelStrategy::Incremental {
                partition_column,
                granularity,
            } => format!(
                "incremental (partition: {}, granularity: {})",
                partition_column, granularity
            ),
            smelt_runtime::ModelStrategy::Cumulative => "cumulative_aggregate".to_string(),
            smelt_runtime::ModelStrategy::Ephemeral => "ephemeral".to_string(),
            smelt_runtime::ModelStrategy::Skipped { reason } => {
                format!("skipped ({})", reason)
            }
        };

        let model_target = graph
            .get_model(model_name)
            .ok()
            .map(|m| config.get_target(model_name, m.metadata.as_deref(), target))
            .unwrap_or_else(|| target.to_string());

        nodes.insert(
            model_name.clone(),
            ExplainPhysicalNode {
                strategy,
                materialization: record.materialization.clone(),
                target: model_target,
                logical_origins: vec![model_name.clone()],
            },
        );
    }

    // Any ephemeral-only models from the graph that aren't in the PlanSummary
    // (e.g., if PlanSummary omitted them) — scan the graph for completeness.
    for (model_name, _) in graph.iter_models() {
        let mat = graph
            .get_model(model_name)
            .ok()
            .map(|m| config.get_materialization_with_metadata(model_name, m.metadata.as_deref()))
            .unwrap_or(Materialization::View);
        if mat == Materialization::Ephemeral && !ephemerals.contains(&model_name.to_string()) {
            ephemerals.push(model_name.to_string());
        }
    }

    let execution_order: Vec<String> = plan_summary
        .models
        .iter()
        .filter(|r| !matches!(r.strategy, smelt_runtime::ModelStrategy::Ephemeral))
        .map(|r| r.name.clone())
        .collect();

    ExplainPhysical {
        execution_order,
        nodes,
        ephemerals,
        transformations: vec![],
    }
}

/// Derive per-source bounds for a model.
fn compute_source_bounds(
    model_name: &str,
    sql: &str,
    graph: &DependencyGraph,
    config: &Config,
) -> BTreeMap<String, SourceBoundJson> {
    use smelt_planner::analysis::source_bounds::derive_model_bounds;
    use smelt_planner::Frontmatter;

    let mut ctx = BoundContext::new();
    for dep_name in graph.get_upstream(model_name) {
        if let Ok(dep_model) = graph.get_model(&dep_name) {
            let dep_meta = dep_model.metadata.as_deref();
            let ts = config
                .get_timeseries_with_metadata(&dep_name, dep_meta)
                .cloned()
                .or_else(|| dep_meta.and_then(|m| m.timeseries.clone()));
            if let Some(ts) = ts {
                ctx.add_source(&dep_name, &ts.partition_column);
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
    inc: &BatchedConfig,
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
                smelt_ref: smelt_core::refs::SmeltRef::Path(vec![dep.to_string()]),
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
                settings: None,
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
            state: Default::default(),
        }
    }

    #[test]
    fn test_batch_safety_uses_expanded_function_body() {
        use smelt_core::config::TimeseriesConfig;
        use smelt_core::{BatchedConfig, Granularity};

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
                refresh: Some(smelt_core::config::RefreshStrategy::Batched),
                batched: Some(BatchedConfig {
                    unique_key: vec![],
                    safety_overrides: Default::default(),
                }),
                tags: vec![],
                target: None,
                format: None,
            },
        )]);
        let graph = DependencyGraph::build(models, None).unwrap();

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
            build_explain_output(&graph, &config, fns, &HashMap::new())
                .unwrap()
                .models["sessions"]
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
                "SELECT date, SUM(amount) FROM smelt.orders GROUP BY date",
            ),
        ];
        let config = make_config(vec![]);
        let graph = DependencyGraph::build(models, None).unwrap();

        let output =
            build_explain_output(&graph, &config, &HashMap::new(), &HashMap::new()).unwrap();

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
        use smelt_core::{BatchedConfig, Granularity};

        let models = vec![
            make_model("orders", vec![], "SELECT * FROM raw_orders"),
            make_model(
                "daily_revenue",
                vec!["orders"],
                "SELECT date, SUM(amount) FROM smelt.orders GROUP BY date",
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
                refresh: Some(smelt_core::config::RefreshStrategy::Batched),
                batched: Some(BatchedConfig {
                    unique_key: vec![],
                    safety_overrides: Default::default(),
                }),
                tags: vec!["revenue".to_string(), "daily".to_string()],
                target: None,
                format: None,
            },
        )]);
        let graph = DependencyGraph::build(models, None).unwrap();

        let output =
            build_explain_output(&graph, &config, &HashMap::new(), &HashMap::new()).unwrap();

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
        let graph = DependencyGraph::build(models, None).unwrap();

        let output =
            build_explain_output(&graph, &config, &HashMap::new(), &HashMap::new()).unwrap();
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
        let config = make_config(vec![]);
        let graph = DependencyGraph::build(models, None).unwrap();

        let output =
            build_explain_output(&graph, &config, &HashMap::new(), &HashMap::new()).unwrap();
        assert_eq!(
            output.models["orders"].owner.as_deref(),
            Some("analytics-team")
        );
    }

    /// `smelt explain --json` must emit `"materialization": "table"` and
    /// `"refresh": "cumulative"` for a cumulative model, and must NOT emit
    /// `"cumulative_aggregate"` anywhere in the materialization field.
    ///
    /// Spec oracle: `docs/specs/cli.md` §"`smelt explain --json` output schema".
    #[test]
    fn explain_json_emits_refresh_cumulative_for_cumulative_model() {
        use crate::metadata::ModelMetadata;
        use smelt_core::config::RefreshStrategy;

        let mut model = make_model(
            "device_stats",
            vec![],
            "SELECT device_id, COUNT(*) AS n FROM smelt.events GROUP BY device_id",
        );
        model.metadata = Some(Box::new(ModelMetadata {
            materialization: Some(Materialization::Table),
            refresh: Some(RefreshStrategy::Cumulative),
            ..Default::default()
        }));

        let models = vec![model];
        let config = make_config(vec![]);
        let graph = DependencyGraph::build(models, None).unwrap();

        let output =
            build_explain_output(&graph, &config, &HashMap::new(), &HashMap::new()).unwrap();

        let model_entry = &output.models["device_stats"];

        // The `materialization` field must be `table` (the storage kind).
        assert_eq!(
            model_entry.materialization,
            Materialization::Table,
            "cumulative model materialization must be 'table', not anything else"
        );

        // The `refresh` field must be `Some(Cumulative)`.
        assert_eq!(
            model_entry.refresh,
            Some(RefreshStrategy::Cumulative),
            "cumulative model must have refresh: Some(Cumulative)"
        );

        // Verify the JSON serialization: must emit `"refresh": "cumulative"`
        // and `"materialization": "table"`, must NOT contain `"cumulative_aggregate"`.
        let json = serde_json::to_string_pretty(&output).unwrap();
        assert!(
            json.contains("\"refresh\": \"cumulative\""),
            "JSON must contain '\"refresh\": \"cumulative\"'; got:\n{json}"
        );
        assert!(
            json.contains("\"materialization\": \"table\""),
            "JSON must contain '\"materialization\": \"table\"'; got:\n{json}"
        );
        assert!(
            !json.contains("\"cumulative_aggregate\""),
            "JSON must not contain '\"cumulative_aggregate\"' in the materialization field; got:\n{json}"
        );
    }

    /// A plain `materialization: table` model (no `refresh: cumulative`) must
    /// NOT emit a `refresh` field in the JSON — the field is omitted for
    /// the default full-refresh strategy.
    #[test]
    fn explain_json_omits_refresh_for_full_refresh_model() {
        let mut model = make_model("orders", vec![], "SELECT * FROM raw_orders");
        model.metadata = Some(Box::new(crate::metadata::ModelMetadata {
            materialization: Some(Materialization::Table),
            refresh: None,
            ..Default::default()
        }));

        let models = vec![model];
        let config = make_config(vec![]);
        let graph = DependencyGraph::build(models, None).unwrap();

        let output =
            build_explain_output(&graph, &config, &HashMap::new(), &HashMap::new()).unwrap();

        let model_entry = &output.models["orders"];
        assert_eq!(
            model_entry.refresh, None,
            "full-refresh model must have no refresh field"
        );

        let json = serde_json::to_string_pretty(&output).unwrap();
        assert!(
            !json.contains("\"refresh\""),
            "full-refresh model JSON must not emit a 'refresh' field; got:\n{json}"
        );
    }
}
