use std::collections::HashMap;

use chrono::{Duration, NaiveDate};
use smelt_core::config::Config;
use smelt_core::graph::DependencyGraph;
use smelt_core::parse_selector;
use smelt_core::SourcesConfig;
use smelt_db::{ColumnSource, DiagnosticSeverity};

use crate::types::*;

pub fn build_project_response(
    config: &Config,
    graph: &DependencyGraph,
    sources: Option<&SourcesConfig>,
) -> ProjectResponse {
    let source_count = sources
        .map(|s| s.sources.iter().map(|src| src.tables.len()).sum())
        .unwrap_or(0);

    ProjectResponse {
        name: config.name.clone(),
        version: config.version,
        model_count: graph.model_count(),
        source_count,
    }
}

pub fn build_graph_response(graph: &DependencyGraph, config: &Config) -> GraphResponse {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    for (name, model) in graph.iter_models() {
        let metadata = model.metadata.as_deref();
        let tags = config.get_tags(name, metadata);

        let is_test = metadata
            .and_then(|m| m.materialization.as_ref())
            .map(|m| *m == smelt_core::config::Materialization::Test)
            .unwrap_or(false);

        let node_type = if is_test {
            NodeType::Test
        } else {
            NodeType::Model
        };

        nodes.push(GraphNode {
            id: name.to_string(),
            label: name.to_string(),
            materialization: metadata
                .and_then(|m| m.materialization.as_ref())
                .map(|m| format!("{:?}", m).to_lowercase()),
            tags,
            description: metadata.and_then(|m| m.description.clone()),
            has_errors: !model.parse_errors.is_empty(),
            node_type,
        });

        // Link test nodes to the model they test
        if is_test {
            if let Some(test_config) = metadata.and_then(|m| m.test.as_ref()) {
                edges.push(GraphEdge {
                    source: test_config.model.clone(),
                    target: name.to_string(),
                });
            }
        }
    }

    for source_name in graph.iter_sources() {
        nodes.push(GraphNode {
            id: source_name.to_string(),
            label: source_name.to_string(),
            materialization: Some("source".to_string()),
            tags: vec![],
            description: None,
            has_errors: false,
            node_type: NodeType::Source,
        });
    }

    for (model_name, deps) in graph.iter_dependencies() {
        for dep in deps {
            edges.push(GraphEdge {
                source: dep.clone(),
                target: model_name.to_string(),
            });
        }
    }

    let graph_sources: Vec<String> = graph.iter_sources().map(|s| s.to_string()).collect();

    GraphResponse {
        nodes,
        edges,
        sources: graph_sources,
    }
}

pub fn build_model_details(
    graph: &DependencyGraph,
    config: &Config,
    db: &smelt_db::Database,
) -> HashMap<String, ModelDetailResponse> {
    let mut model_details: HashMap<String, ModelDetailResponse> = HashMap::new();

    let ws = smelt_db::Workspace::try_get(db);

    for (name, model) in graph.iter_models() {
        let metadata = model.metadata.as_deref();
        let tags = config.get_tags(name, metadata);

        let file = db.source_file(&model.path);
        let schema = match (ws, file) {
            (Some(w), Some(f)) => smelt_db::typed_model_schema(db, w, f),
            _ => std::sync::Arc::new(smelt_db::ModelSchema::empty()),
        };
        let function_type = match (ws, file) {
            (Some(w), Some(f)) => Some(smelt_db::model_function_type(db, w, f).to_string()),
            _ => None,
        };
        let columns: Vec<ColumnInfo> = schema
            .columns
            .iter()
            .filter(|col| col.name != "*")
            .map(|col| {
                let (data_type, nullable) = match &col.data_type {
                    Some(t) => (Some(t.data_type.to_string()), Some(t.nullable)),
                    None => (None, None),
                };

                let source = match &col.source {
                    ColumnSource::Computed => ColumnSourceInfo::Computed,
                    ColumnSource::FromModel {
                        model_name,
                        column_name,
                    } => ColumnSourceInfo::FromModel {
                        model: model_name.clone(),
                        column: column_name.clone(),
                    },
                    ColumnSource::Wildcard { model_name } => ColumnSourceInfo::Wildcard {
                        model: model_name.clone(),
                    },
                    ColumnSource::ExternalTable { table_name } => ColumnSourceInfo::ExternalTable {
                        table: table_name.clone(),
                    },
                    ColumnSource::Unknown => ColumnSourceInfo::Unknown,
                };

                ColumnInfo {
                    name: col.name.clone(),
                    data_type,
                    nullable,
                    source,
                    expression: col.expression.clone(),
                }
            })
            .collect();

        // Build incremental info from config
        let inc_config = config
            .get_incremental(name)
            .or_else(|| metadata.and_then(|m| m.incremental.as_ref()));
        let ts_config = config
            .get_timeseries_with_metadata(name, metadata)
            .or_else(|| metadata.and_then(|m| m.timeseries.as_ref()));

        let incremental = inc_config.and_then(|inc| {
            ts_config.map(|ts| IncrementalInfo {
                granularity: format!("{:?}", ts.granularity).to_lowercase(),
                partition_column: ts.partition_column.clone(),
                event_time_column: ts.event_time_column.clone(),
                unique_key: inc.unique_key.clone(),
            })
        });

        // Build batch safety info
        let batch_safety = inc_config.and_then(|inc| {
            ts_config.map(|ts| {
                use smelt_planner::analyze_batch_safety;
                use smelt_planner::ModelInfo;

                let model_info = ModelInfo {
                    name: name.to_string(),
                    sql: model.content.clone(),
                    refs: model.refs.iter().map(|r| r.model_name.clone()).collect(),
                    timeseries_config: Some(ts.clone()),
                    incremental_config: Some(inc.clone()),
                };
                let safety = analyze_batch_safety(&model_info);
                match safety {
                    smelt_planner::BatchSafety::FullyBatchSafe => BatchSafetyInfo {
                        level: "fully_batch_safe".to_string(),
                        max_chunk_days: None,
                        context_days: None,
                        reason: None,
                    },
                    smelt_planner::BatchSafety::BoundedSafe {
                        max_chunk_days,
                        context_days,
                        reason,
                    } => BatchSafetyInfo {
                        level: "bounded_safe".to_string(),
                        max_chunk_days: Some(max_chunk_days),
                        context_days: Some(context_days),
                        reason: Some(reason),
                    },
                    smelt_planner::BatchSafety::PerPartitionOnly { reason } => BatchSafetyInfo {
                        level: "per_partition_only".to_string(),
                        max_chunk_days: None,
                        context_days: None,
                        reason: Some(reason),
                    },
                }
            }) // close ts_config.map
        });

        // Build diagnostics
        let diags = match (ws, file) {
            (Some(w), Some(f)) => smelt_db::file_diagnostics(db, w, f),
            _ => Vec::new(),
        };
        let diagnostics: Vec<DiagnosticInfo> = diags
            .iter()
            .map(|d| DiagnosticInfo {
                severity: match d.severity {
                    DiagnosticSeverity::Error => "error".to_string(),
                    DiagnosticSeverity::Warning => "warning".to_string(),
                    DiagnosticSeverity::Info => "info".to_string(),
                    DiagnosticSeverity::Hint => "hint".to_string(),
                },
                message: d.message.clone(),
                line: Some(d.range.start.line),
                column: Some(d.range.start.column),
            })
            .collect();

        model_details.insert(
            name.to_string(),
            ModelDetailResponse {
                name: name.to_string(),
                path: model.path.display().to_string(),
                sql: model.content.clone(),
                materialization: metadata
                    .and_then(|m| m.materialization.as_ref())
                    .map(|m| format!("{:?}", m).to_lowercase()),
                tags,
                owner: metadata.and_then(|m| m.owner.clone()),
                description: metadata.and_then(|m| m.description.clone()),
                refs: model.refs.iter().map(|r| r.model_name.clone()).collect(),
                columns,
                incremental,
                batch_safety,
                diagnostics,
                function_type,
            },
        );
    }

    model_details
}

/// Resolve select/exclude selectors to model names without computing a full plan.
pub fn resolve_selectors(
    graph: &DependencyGraph,
    config: &Config,
    request: &crate::types::ResolveRequest,
) -> anyhow::Result<crate::types::ResolveResponse> {
    let selected_set = if request.select.is_empty() {
        std::collections::HashSet::new()
    } else {
        let selectors: Vec<_> = request
            .select
            .iter()
            .map(|s| {
                parse_selector(s).map_err(|e| anyhow::anyhow!("Invalid selector '{}': {}", s, e))
            })
            .collect::<Result<_, _>>()?;
        graph.select_models(&selectors, config)?
    };

    let excluded_set = if request.exclude.is_empty() {
        std::collections::HashSet::new()
    } else {
        let excludes: Vec<_> = request
            .exclude
            .iter()
            .map(|s| {
                parse_selector(s)
                    .map_err(|e| anyhow::anyhow!("Invalid exclude selector '{}': {}", s, e))
            })
            .collect::<Result<_, _>>()?;
        graph.select_models(&excludes, config)?
    };

    // Return in execution order for consistency
    let all_order = graph.execution_order()?;
    let selected: Vec<String> = all_order
        .iter()
        .filter(|n| selected_set.contains(*n))
        .cloned()
        .collect();
    let excluded: Vec<String> = all_order
        .iter()
        .filter(|n| excluded_set.contains(*n))
        .cloned()
        .collect();

    Ok(crate::types::ResolveResponse { selected, excluded })
}

/// Compute a run plan preview — what models would run and how they'd be batched.
pub fn build_run_plan(
    graph: &DependencyGraph,
    config: &Config,
    request: &crate::types::RunPlanRequest,
) -> anyhow::Result<crate::types::RunPlanResponse> {
    use smelt_planner::{analyze_batch_safety, BatchSafety, Frontmatter, ModelInfo};

    let start = NaiveDate::parse_from_str(&request.start, "%Y-%m-%d")
        .map_err(|_| anyhow::anyhow!("Invalid start date: {}", request.start))?;
    let end = NaiveDate::parse_from_str(&request.end, "%Y-%m-%d")
        .map_err(|_| anyhow::anyhow!("Invalid end date: {}", request.end))?;

    if start >= end {
        return Err(anyhow::anyhow!("Start date must be before end date"));
    }

    // Use the shared selection pass so the preview (`/api/run/plan`) lists
    // the same models that `/api/run/execute` would actually execute —
    // tests filtered, generators filtered, selectors resolved consistently.
    // Target assignments are computed but unused here; the preview cares
    // only about the model list and batch shapes.
    let selection_request = smelt_runtime::SelectionRequest {
        select: request.select.clone(),
        exclude: request.exclude.clone(),
        target: String::new(),
    };
    let selected: Vec<String> =
        smelt_runtime::select_executable_models(graph, config, &selection_request)?.ordered_models;

    let mut plan_models = Vec::new();
    let mut total_batches = 0;

    for model_name in &selected {
        let model = match graph.get_model(model_name) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let metadata = model.metadata.as_deref();
        let frontmatter = Frontmatter::parse(&model.content);

        let inc_config = config
            .get_incremental_with_metadata(model_name, metadata)
            .cloned()
            .or_else(|| frontmatter.as_ref().and_then(|f| f.incremental.clone()));

        let ts_config = config
            .get_timeseries_with_metadata(model_name, metadata)
            .cloned()
            .or_else(|| metadata.and_then(|m| m.timeseries.clone()));

        match (inc_config, ts_config) {
            (Some(inc), Some(ts)) => {
                let model_info = ModelInfo {
                    name: model_name.clone(),
                    sql: model.content.clone(),
                    refs: model.refs.iter().map(|r| r.model_name.clone()).collect(),
                    timeseries_config: Some(ts.clone()),
                    incremental_config: Some(inc.clone()),
                };
                let safety = analyze_batch_safety(&model_info);

                let batch_safety_info = match &safety {
                    BatchSafety::FullyBatchSafe => BatchSafetyInfo {
                        level: "fully_batch_safe".to_string(),
                        max_chunk_days: None,
                        context_days: None,
                        reason: None,
                    },
                    BatchSafety::BoundedSafe {
                        max_chunk_days,
                        context_days,
                        reason,
                    } => BatchSafetyInfo {
                        level: "bounded_safe".to_string(),
                        max_chunk_days: Some(*max_chunk_days),
                        context_days: Some(*context_days),
                        reason: Some(reason.clone()),
                    },
                    BatchSafety::PerPartitionOnly { reason } => BatchSafetyInfo {
                        level: "per_partition_only".to_string(),
                        max_chunk_days: None,
                        context_days: None,
                        reason: Some(reason.clone()),
                    },
                };

                // Generate batches
                let (batch_days, context_days) = if request.per_partition {
                    (granularity_days(&ts.granularity), 0)
                } else if let Some(override_days) = request.batch_size_days {
                    let ctx = match &safety {
                        BatchSafety::BoundedSafe { context_days, .. } => *context_days,
                        _ => 0,
                    };
                    (override_days, ctx)
                } else {
                    match &safety {
                        BatchSafety::FullyBatchSafe => ((end - start).num_days() as u32, 0),
                        BatchSafety::BoundedSafe {
                            max_chunk_days,
                            context_days,
                            ..
                        } => (*max_chunk_days, *context_days),
                        BatchSafety::PerPartitionOnly { .. } => {
                            (granularity_days(&ts.granularity), 0)
                        }
                    }
                };

                let mut batches = Vec::new();
                let mut batch_start = start;
                while batch_start < end {
                    let batch_end = (batch_start + Duration::days(batch_days as i64)).min(end);
                    let filter_start = batch_start - Duration::days(context_days as i64);

                    batches.push(crate::types::PlanBatch {
                        partition_start: batch_start.format("%Y-%m-%d").to_string(),
                        partition_end: batch_end.format("%Y-%m-%d").to_string(),
                        filter_start: filter_start.format("%Y-%m-%d").to_string(),
                        filter_end: batch_end.format("%Y-%m-%d").to_string(),
                    });
                    batch_start = batch_end;
                }

                total_batches += batches.len();

                plan_models.push(crate::types::PlanModel {
                    name: model_name.clone(),
                    is_incremental: true,
                    batch_safety: Some(batch_safety_info),
                    partition_range: Some(crate::types::PlanTimeRange {
                        start: request.start.clone(),
                        end: request.end.clone(),
                    }),
                    filter_range: Some(crate::types::PlanTimeRange {
                        start: (start - Duration::days(context_days as i64))
                            .format("%Y-%m-%d")
                            .to_string(),
                        end: request.end.clone(),
                    }),
                    batches,
                });
            }
            _ => {
                // Non-incremental model (or incremental without timeseries) — full refresh
                plan_models.push(crate::types::PlanModel {
                    name: model_name.clone(),
                    is_incremental: false,
                    batch_safety: None,
                    partition_range: None,
                    filter_range: None,
                    batches: vec![],
                });
            }
        }
    }

    let cli_command = build_cli_command(request);

    Ok(crate::types::RunPlanResponse {
        models: plan_models,
        execution_order: selected,
        total_batches,
        cli_command,
    })
}

fn build_cli_command(request: &crate::types::RunPlanRequest) -> String {
    let mut parts = vec![format!(
        "smelt run --start {} --end {}",
        request.start, request.end
    )];

    for s in &request.select {
        parts.push(format!("--select {}", s));
    }
    for e in &request.exclude {
        parts.push(format!("--exclude {}", e));
    }
    if let Some(bs) = request.batch_size_days {
        parts.push(format!("--batch-size {}", bs));
    }
    if request.per_partition {
        parts.push("--per-partition".to_string());
    }

    parts.join(" ")
}

fn granularity_days(g: &smelt_core::Granularity) -> u32 {
    match g {
        smelt_core::Granularity::Hour => 1,
        smelt_core::Granularity::Day => 1,
        smelt_core::Granularity::Week => 7,
        smelt_core::Granularity::Month => 30,
        smelt_core::Granularity::Quarter => 91,
        smelt_core::Granularity::Year => 365,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rowan::TextRange;
    use smelt_core::config::{Materialization, Target};
    use smelt_core::discovery::ModelFile;
    use smelt_core::refs::RefInfo;
    use smelt_core::{SourceColumnDef, SourceDef, SourceTableDef};
    use smelt_db::SourceFile;

    fn make_model(name: &str, deps: Vec<&str>) -> ModelFile {
        let refs = deps
            .into_iter()
            .map(|dep| RefInfo {
                model_name: dep.to_string(),
                has_named_params: false,
                range: TextRange::default(),
                smelt_ref: smelt_core::refs::SmeltRef::Path(vec![
                    "models".to_string(),
                    dep.to_string(),
                ]),
            })
            .collect();

        let path: std::path::PathBuf = format!("{}.sql", name).into();
        ModelFile {
            name: name.to_string(),
            model_id: smelt_core::ModelId::from_path(path.clone()),
            path,
            content: format!("SELECT * FROM {}", name),
            refs,
            parse_errors: Vec::new(),
            metadata: None,
            kind: smelt_core::ModelKind::Sql,
            // TODO Phase 5: compute address_segments from the synthetic model
            // name so canonical_path() returns the correct dot-path.
            address_segments: Vec::new(),
        }
    }

    fn make_test_config() -> Config {
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
            name: "test-project".to_string(),
            version: 1,
            paths: vec!["models".to_string()],
            targets,
            default_materialization: Materialization::View,
            models: HashMap::new(),
            python: None,
        }
    }

    #[test]
    fn test_build_project_response() {
        let config = make_test_config();
        let models = vec![make_model("A", vec![]), make_model("B", vec!["A"])];
        let graph = DependencyGraph::build(models, None).unwrap();

        let response = build_project_response(&config, &graph, None);

        assert_eq!(response.name, "test-project");
        assert_eq!(response.version, 1);
        assert_eq!(response.model_count, 2);
        assert_eq!(response.source_count, 0);
    }

    #[test]
    fn test_build_project_response_with_sources() {
        let config = make_test_config();
        let models = vec![make_model("A", vec![])];
        let graph = DependencyGraph::build(models, None).unwrap();

        let sources = SourcesConfig {
            sources: vec![SourceDef {
                name: "raw".to_string(),
                database: None,
                schema: None,
                description: None,
                tables: vec![
                    SourceTableDef {
                        name: "events".to_string(),
                        identifier: None,
                        description: None,
                        columns: vec![SourceColumnDef {
                            name: "id".to_string(),
                            data_type: None,
                            description: None,
                            data_latency: None,
                        }],
                    },
                    SourceTableDef {
                        name: "users".to_string(),
                        identifier: None,
                        description: None,
                        columns: vec![],
                    },
                ],
            }],
        };

        let response = build_project_response(&config, &graph, Some(&sources));

        assert_eq!(response.source_count, 2);
    }

    #[test]
    fn test_build_graph_response_nodes() {
        let config = make_test_config();
        let source_config = SourcesConfig {
            sources: vec![SourceDef {
                name: "raw".to_string(),
                database: None,
                schema: None,
                description: None,
                tables: vec![SourceTableDef {
                    name: "events".to_string(),
                    identifier: None,
                    description: None,
                    columns: vec![],
                }],
            }],
        };

        let models = vec![make_model("A", vec!["raw.events"])];
        let graph = DependencyGraph::build(models, Some(&source_config)).unwrap();

        let response = build_graph_response(&graph, &config);

        // Should have 1 model node + 1 source node
        assert_eq!(response.nodes.len(), 2);

        let model_node = response.nodes.iter().find(|n| n.id == "A").unwrap();
        assert!(matches!(model_node.node_type, NodeType::Model));

        let source_node = response
            .nodes
            .iter()
            .find(|n| n.id == "raw.events")
            .unwrap();
        assert!(matches!(source_node.node_type, NodeType::Source));
        assert_eq!(source_node.materialization, Some("source".to_string()));
    }

    #[test]
    fn test_build_graph_response_edges() {
        let config = make_test_config();
        let models = vec![make_model("A", vec![]), make_model("B", vec!["A"])];
        let graph = DependencyGraph::build(models, None).unwrap();

        let response = build_graph_response(&graph, &config);

        assert_eq!(response.edges.len(), 1);
        assert_eq!(response.edges[0].source, "A");
        assert_eq!(response.edges[0].target, "B");
    }

    #[test]
    fn test_build_model_details_columns() {
        let config = make_test_config();
        let models = vec![make_model("A", vec![])];
        let graph = DependencyGraph::build(models.clone(), None).unwrap();

        let mut db = smelt_db::Database::default();
        let project_root = std::path::PathBuf::from("/test");
        let project = db.set_project_input(project_root.clone(), String::new());

        let mut source_files: Vec<SourceFile> = Vec::new();
        for model in &models {
            let sf = db.set_source_file(
                model.path.clone(),
                model.content.clone(),
                project_root.clone(),
            );
            source_files.push(sf);
        }
        db.set_workspace(source_files, vec![project]);

        let details = build_model_details(&graph, &config, &db);

        assert!(details.contains_key("A"));
        let a = &details["A"];
        assert_eq!(a.name, "A");
        assert_eq!(a.sql, "SELECT * FROM A");
        // No incremental config → should be None
        assert!(a.incremental.is_none());
        assert!(a.batch_safety.is_none());
    }
}
