use std::collections::HashMap;

use smelt_core::config::Config;
use smelt_core::graph::DependencyGraph;
use smelt_core::SourcesConfig;
use smelt_db::{ColumnSource, TypeChecking};

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

        nodes.push(GraphNode {
            id: name.to_string(),
            label: name.to_string(),
            materialization: metadata
                .and_then(|m| m.materialization.as_ref())
                .map(|m| format!("{:?}", m).to_lowercase()),
            tags,
            description: metadata.and_then(|m| m.description.clone()),
            has_errors: !model.parse_errors.is_empty(),
            node_type: NodeType::Model,
        });
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

    for (name, model) in graph.iter_models() {
        let metadata = model.metadata.as_deref();
        let tags = config.get_tags(name, metadata);

        let schema = db.typed_model_schema(model.path.clone());
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
            },
        );
    }

    model_details
}

#[cfg(test)]
mod tests {
    use super::*;
    use rowan::TextRange;
    use smelt_core::config::{Materialization, Target};
    use smelt_core::discovery::ModelFile;
    use smelt_core::refs::RefInfo;
    use smelt_core::{SourceColumnDef, SourceDef, SourceTableDef};
    use smelt_db::Inputs;
    use std::sync::Arc;

    fn make_model(name: &str, deps: Vec<&str>) -> ModelFile {
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
            content: format!("SELECT * FROM {}", name),
            refs,
            parse_errors: Vec::new(),
            metadata: None,
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
            },
        );

        Config {
            name: "test-project".to_string(),
            version: 1,
            model_paths: vec!["models".to_string()],
            seed_paths: vec!["seeds".to_string()],
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
        db.set_project_sources_yaml(project_root.clone(), Arc::new(String::new()));
        db.set_all_project_roots(Arc::new(vec![project_root.clone()]));

        let mut file_paths = Vec::new();
        for model in &models {
            db.set_file_text(model.path.clone(), Arc::new(model.content.clone()));
            db.set_file_project_root(model.path.clone(), project_root.clone());
            file_paths.push(model.path.clone());
        }
        db.set_all_files(Arc::new(file_paths));

        let details = build_model_details(&graph, &config, &db);

        assert!(details.contains_key("A"));
        let a = &details["A"];
        assert_eq!(a.name, "A");
        assert_eq!(a.sql, "SELECT * FROM A");
    }
}
