use crate::compiler::{resolve_refs_in_sql, CompilerRegistry, EphemeralResolver};
use crate::discovery::ModelFile;
use crate::logical_graph::LogicalGraph;
use crate::transformer::TimeRange;
use anyhow::Result;
use smelt_core::config::{IncrementalConfig, Materialization};
use smelt_planner::{ExecutionStep, Transformation};
use std::collections::HashMap;

/// How a physical node should be executed.
#[derive(Debug, Clone)]
pub enum PhysicalStrategy {
    /// Full refresh: DROP + CREATE TABLE AS or CREATE VIEW AS.
    FullRefresh,
    /// Multi-step plan without incremental (e.g., cube split).
    CubeSplit { steps: Vec<ExecutionStep> },
    /// Incremental processing (with optional cube split steps).
    Incremental {
        config: IncrementalConfig,
        time_range: TimeRange,
        plan_steps: Option<Vec<ExecutionStep>>,
    },
}

/// A node in the physical execution graph.
///
/// Each PhysicalNode corresponds to a non-ephemeral model that needs execution.
/// Ephemeral models are absorbed into EphemeralResolvers during construction.
pub struct PhysicalNode {
    /// Model name (same as LogicalNode.name for user-authored models).
    pub name: String,
    /// Reference to the model file (for compilation).
    pub model_file: ModelFile,
    /// How this model should be materialized (Table, View, MaterializedView).
    pub materialization: Materialization,
    /// Which backend target to execute on.
    pub target: String,
    /// Resolved execution strategy.
    pub strategy: PhysicalStrategy,
}

/// The physical execution graph — what the executor consumes.
///
/// Contains only executable nodes (no ephemerals) in topological order,
/// with pre-built ephemeral resolvers per target.
pub struct PhysicalGraph {
    nodes: HashMap<String, PhysicalNode>,
    execution_order: Vec<String>,
    ephemeral_resolvers: HashMap<String, EphemeralResolver>,
}

impl PhysicalGraph {
    pub fn get_node(&self, name: &str) -> Option<&PhysicalNode> {
        self.nodes.get(name)
    }

    pub fn execution_order(&self) -> &[String] {
        &self.execution_order
    }

    /// Get the ephemeral resolver for a target, or an empty one if none exists.
    pub fn ephemeral_resolver(&self, target: &str) -> &EphemeralResolver {
        static EMPTY: std::sync::OnceLock<EphemeralResolver> = std::sync::OnceLock::new();
        self.ephemeral_resolvers
            .get(target)
            .unwrap_or_else(|| EMPTY.get_or_init(EphemeralResolver::empty))
    }

    /// Iterate over physical nodes in execution order.
    pub fn iter_in_order(&self) -> impl Iterator<Item = &PhysicalNode> {
        self.execution_order
            .iter()
            .filter_map(|name| self.nodes.get(name))
    }

    /// Whether any node has an incremental strategy from planner overrides.
    ///
    /// Used by the caller to check if a time_range is mandatory.
    pub fn has_planner_incremental(&self) -> bool {
        self.nodes.values().any(|node| {
            matches!(
                node.strategy,
                PhysicalStrategy::Incremental {
                    plan_steps: None,
                    ..
                }
            )
        })
    }

    /// Summary of planner-applied strategies for display.
    pub fn planner_summary(&self) -> Vec<(String, String)> {
        let mut summary = Vec::new();
        for node in self.iter_in_order() {
            match &node.strategy {
                PhysicalStrategy::CubeSplit { .. } => {
                    summary.push((node.name.clone(), "cube split".to_string()));
                }
                PhysicalStrategy::Incremental { config, .. } => {
                    summary.push((
                        node.name.clone(),
                        format!("incremental (partition: {})", config.partition_column),
                    ));
                }
                PhysicalStrategy::FullRefresh => {}
            }
        }
        summary
    }
}

/// Builds a PhysicalGraph from a LogicalGraph + planner transformations.
pub struct PhysicalGraphBuilder<'a> {
    logical_graph: &'a LogicalGraph,
    transformations: &'a [Transformation],
    time_range: Option<TimeRange>,
    compilers: &'a CompilerRegistry,
    target_schemas: HashMap<String, String>,
}

impl<'a> PhysicalGraphBuilder<'a> {
    pub fn new(
        logical_graph: &'a LogicalGraph,
        transformations: &'a [Transformation],
        time_range: Option<TimeRange>,
        compilers: &'a CompilerRegistry,
        target_schemas: HashMap<String, String>,
    ) -> Self {
        Self {
            logical_graph,
            transformations,
            time_range,
            compilers,
            target_schemas,
        }
    }

    pub fn build(self) -> Result<PhysicalGraph> {
        // 1. Parse transformations into lookup maps
        let mut plan_overrides: HashMap<String, Vec<ExecutionStep>> = HashMap::new();
        let mut incremental_overrides: HashMap<String, (String, String, smelt_core::Granularity)> =
            HashMap::new();

        for t in self.transformations {
            match t {
                Transformation::ReplaceWithPlan { model, steps } => {
                    plan_overrides.insert(model.clone(), steps.clone());
                }
                Transformation::SetIncremental {
                    model,
                    event_time_column,
                    partition_column,
                    granularity,
                } => {
                    incremental_overrides.insert(
                        model.clone(),
                        (
                            event_time_column.clone(),
                            partition_column.clone(),
                            granularity.clone(),
                        ),
                    );
                }
            }
        }

        // 2. Build ephemeral resolvers per target
        let mut ephemeral_models_by_target: HashMap<String, Vec<(String, String)>> = HashMap::new();

        let execution_order = self.logical_graph.execution_order()?;

        for model_name in &execution_order {
            let node = self.logical_graph.get_node(model_name)?;
            if node.materialization == Materialization::Ephemeral {
                ephemeral_models_by_target
                    .entry(node.target.clone())
                    .or_default()
                    .push((model_name.clone(), node.model_file.content.clone()));
            }
        }

        let mut ephemeral_resolvers: HashMap<String, EphemeralResolver> = HashMap::new();
        for (target_name, models) in &ephemeral_models_by_target {
            let compiler = self.compilers.get(target_name);
            let schema = &self.target_schemas[target_name];
            ephemeral_resolvers.insert(
                target_name.clone(),
                compiler.build_ephemeral_resolver(models, schema),
            );
        }

        // 3. Build physical nodes for non-ephemeral models
        let mut nodes = HashMap::new();
        let mut physical_order = Vec::new();

        for model_name in &execution_order {
            let node = self.logical_graph.get_node(model_name)?;

            // Skip ephemeral models — they are inlined as CTEs
            if node.materialization == Materialization::Ephemeral {
                continue;
            }

            let schema = &self.target_schemas[&node.target];

            // Resolve plan steps with ref resolution
            let plan_steps = plan_overrides.get(model_name.as_str()).map(|steps| {
                steps
                    .iter()
                    .map(|step| match step {
                        ExecutionStep::CreateTemp { name, sql } => ExecutionStep::CreateTemp {
                            name: name.clone(),
                            sql: resolve_refs_in_sql(sql, schema),
                        },
                        ExecutionStep::AppendToTemp { name, sql } => ExecutionStep::AppendToTemp {
                            name: name.clone(),
                            sql: resolve_refs_in_sql(sql, schema),
                        },
                        ExecutionStep::FinalQuery { sql } => {
                            ExecutionStep::FinalQuery { sql: sql.clone() }
                        }
                        ExecutionStep::DropTemp { name } => {
                            ExecutionStep::DropTemp { name: name.clone() }
                        }
                    })
                    .collect::<Vec<_>>()
            });

            // Resolve execution strategy
            let strategy = if let Some((event_time_col, partition_col, granularity)) =
                incremental_overrides.get(model_name.as_str())
            {
                // Planner detected incremental — requires time_range
                if let Some(ref time_range) = self.time_range {
                    PhysicalStrategy::Incremental {
                        config: IncrementalConfig {
                            enabled: true,
                            event_time_column: event_time_col.clone(),
                            partition_column: partition_col.clone(),
                            granularity: granularity.clone(),
                            unique_key: vec![],
                            safety_overrides: Default::default(),
                        },
                        time_range: time_range.clone(),
                        plan_steps,
                    }
                } else if let Some(steps) = plan_steps {
                    PhysicalStrategy::CubeSplit { steps }
                } else {
                    PhysicalStrategy::FullRefresh
                }
            } else if let Some(inc) = node
                .incremental
                .as_ref()
                .filter(|_| self.time_range.is_some())
            {
                // Config-based incremental (smelt.yml or frontmatter)
                PhysicalStrategy::Incremental {
                    config: inc.clone(),
                    time_range: self.time_range.as_ref().unwrap().clone(),
                    plan_steps,
                }
            } else if let Some(steps) = plan_steps {
                PhysicalStrategy::CubeSplit { steps }
            } else {
                PhysicalStrategy::FullRefresh
            };

            physical_order.push(model_name.clone());
            nodes.insert(
                model_name.clone(),
                PhysicalNode {
                    name: model_name.clone(),
                    model_file: node.model_file.clone(),
                    materialization: node.materialization.clone(),
                    target: node.target.clone(),
                    strategy,
                },
            );
        }

        Ok(PhysicalGraph {
            nodes,
            execution_order: physical_order,
            ephemeral_resolvers,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::discovery::ModelKind;
    use rowan::TextRange;
    use smelt_core::config::{Granularity, ModelConfig, Target};
    use smelt_core::ModelId;
    use smelt_core::RefInfo;
    use std::collections::HashMap;

    fn make_model(name: &str, deps: Vec<&str>) -> ModelFile {
        let refs = deps
            .into_iter()
            .map(|dep| RefInfo {
                model_name: dep.to_string(),
                has_named_params: false,
                range: TextRange::default(),
            })
            .collect();

        let path: std::path::PathBuf = format!("{}.sql", name).into();
        ModelFile {
            name: name.to_string(),
            model_id: ModelId::from_path(path.clone()),
            path,
            content: format!("SELECT 1 AS id -- {}", name),
            refs,
            parse_errors: Vec::new(),
            metadata: None,
            kind: ModelKind::Sql,
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
            name: "test".to_string(),
            version: 1,
            model_paths: vec!["models".to_string()],
            seed_paths: vec!["seeds".to_string()],
            targets,
            default_materialization: Materialization::View,
            models: HashMap::new(),
            python: None,
        }
    }

    fn build_graph_and_compilers(
        models: Vec<ModelFile>,
        config: &Config,
    ) -> (LogicalGraph, CompilerRegistry, HashMap<String, String>) {
        let graph = LogicalGraph::build(models, None, config, "dev").unwrap();
        let target_configs: HashMap<String, Target> = config
            .targets
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let compilers = CompilerRegistry::new(config, &target_configs);
        let target_schemas: HashMap<String, String> = config
            .targets
            .iter()
            .map(|(k, v)| (k.clone(), v.schema.clone()))
            .collect();
        (graph, compilers, target_schemas)
    }

    #[test]
    fn test_full_refresh_strategy() {
        let models = vec![make_model("A", vec![]), make_model("B", vec!["A"])];
        let config = make_test_config();
        let (graph, compilers, schemas) = build_graph_and_compilers(models, &config);

        let pg = PhysicalGraphBuilder::new(&graph, &[], None, &compilers, schemas)
            .build()
            .unwrap();

        assert_eq!(pg.execution_order().len(), 2);
        for node in pg.iter_in_order() {
            assert!(matches!(node.strategy, PhysicalStrategy::FullRefresh));
        }
    }

    #[test]
    fn test_ephemeral_filtered_out() {
        let models = vec![
            make_model("staging", vec![]),
            make_model("mart", vec!["staging"]),
        ];
        let mut config = make_test_config();
        config.models.insert(
            "staging".to_string(),
            ModelConfig {
                materialization: Some(Materialization::Ephemeral),
                incremental: None,
                tags: vec![],
                target: None,
            },
        );

        let (graph, compilers, schemas) = build_graph_and_compilers(models, &config);
        let pg = PhysicalGraphBuilder::new(&graph, &[], None, &compilers, schemas)
            .build()
            .unwrap();

        assert_eq!(pg.execution_order().len(), 1);
        assert_eq!(pg.execution_order()[0], "mart");
        assert!(pg.get_node("staging").is_none());
    }

    #[test]
    fn test_incremental_with_time_range() {
        let models = vec![make_model("events", vec![])];
        let mut config = make_test_config();
        config.models.insert(
            "events".to_string(),
            ModelConfig {
                materialization: Some(Materialization::Table),
                incremental: Some(IncrementalConfig {
                    enabled: true,
                    event_time_column: "ts".to_string(),
                    partition_column: "dt".to_string(),
                    granularity: Granularity::Day,
                    unique_key: vec![],
                    safety_overrides: Default::default(),
                }),
                tags: vec![],
                target: None,
            },
        );

        let time_range = Some(TimeRange {
            start: "2026-01-01".to_string(),
            end: "2026-01-02".to_string(),
        });

        let (graph, compilers, schemas) = build_graph_and_compilers(models, &config);
        let pg = PhysicalGraphBuilder::new(&graph, &[], time_range, &compilers, schemas)
            .build()
            .unwrap();

        let node = pg.get_node("events").unwrap();
        assert!(matches!(
            node.strategy,
            PhysicalStrategy::Incremental { .. }
        ));
    }

    #[test]
    fn test_incremental_without_time_range_falls_back() {
        let models = vec![make_model("events", vec![])];
        let mut config = make_test_config();
        config.models.insert(
            "events".to_string(),
            ModelConfig {
                materialization: Some(Materialization::Table),
                incremental: Some(IncrementalConfig {
                    enabled: true,
                    event_time_column: "ts".to_string(),
                    partition_column: "dt".to_string(),
                    granularity: Granularity::Day,
                    unique_key: vec![],
                    safety_overrides: Default::default(),
                }),
                tags: vec![],
                target: None,
            },
        );

        let (graph, compilers, schemas) = build_graph_and_compilers(models, &config);
        let pg = PhysicalGraphBuilder::new(&graph, &[], None, &compilers, schemas)
            .build()
            .unwrap();

        let node = pg.get_node("events").unwrap();
        assert!(matches!(node.strategy, PhysicalStrategy::FullRefresh));
    }

    #[test]
    fn test_cube_split_strategy() {
        let models = vec![make_model("report", vec![])];
        let config = make_test_config();

        let transformations = vec![Transformation::ReplaceWithPlan {
            model: "report".to_string(),
            steps: vec![
                ExecutionStep::CreateTemp {
                    name: "tmp".to_string(),
                    sql: "SELECT 1".to_string(),
                },
                ExecutionStep::FinalQuery {
                    sql: "SELECT * FROM tmp".to_string(),
                },
                ExecutionStep::DropTemp {
                    name: "tmp".to_string(),
                },
            ],
        }];

        let (graph, compilers, schemas) = build_graph_and_compilers(models, &config);
        let pg = PhysicalGraphBuilder::new(&graph, &transformations, None, &compilers, schemas)
            .build()
            .unwrap();

        let node = pg.get_node("report").unwrap();
        assert!(matches!(node.strategy, PhysicalStrategy::CubeSplit { .. }));
    }

    #[test]
    fn test_planner_incremental_with_time_range() {
        let models = vec![make_model("events", vec![])];
        let config = make_test_config();

        let transformations = vec![Transformation::SetIncremental {
            model: "events".to_string(),
            event_time_column: "ts".to_string(),
            partition_column: "dt".to_string(),
            granularity: Granularity::Day,
        }];

        let time_range = Some(TimeRange {
            start: "2026-01-01".to_string(),
            end: "2026-01-02".to_string(),
        });

        let (graph, compilers, schemas) = build_graph_and_compilers(models, &config);
        let pg =
            PhysicalGraphBuilder::new(&graph, &transformations, time_range, &compilers, schemas)
                .build()
                .unwrap();

        let node = pg.get_node("events").unwrap();
        assert!(matches!(
            node.strategy,
            PhysicalStrategy::Incremental { .. }
        ));
    }

    #[test]
    fn test_ephemeral_resolver_built_per_target() {
        let models = vec![
            make_model("staging", vec![]),
            make_model("mart", vec!["staging"]),
        ];
        let mut config = make_test_config();
        config.models.insert(
            "staging".to_string(),
            ModelConfig {
                materialization: Some(Materialization::Ephemeral),
                incremental: None,
                tags: vec![],
                target: None,
            },
        );

        let (graph, compilers, schemas) = build_graph_and_compilers(models, &config);
        let pg = PhysicalGraphBuilder::new(&graph, &[], None, &compilers, schemas)
            .build()
            .unwrap();

        // The resolver for "dev" target should know about the ephemeral model
        let resolver = pg.ephemeral_resolver("dev");
        assert!(resolver.ephemeral_names.contains("staging"));
    }

    #[test]
    fn test_planner_summary() {
        let models = vec![make_model("report", vec![])];
        let config = make_test_config();

        let transformations = vec![Transformation::ReplaceWithPlan {
            model: "report".to_string(),
            steps: vec![ExecutionStep::FinalQuery {
                sql: "SELECT 1".to_string(),
            }],
        }];

        let (graph, compilers, schemas) = build_graph_and_compilers(models, &config);
        let pg = PhysicalGraphBuilder::new(&graph, &transformations, None, &compilers, schemas)
            .build()
            .unwrap();

        let summary = pg.planner_summary();
        assert_eq!(summary.len(), 1);
        assert_eq!(summary[0].0, "report");
        assert!(summary[0].1.contains("cube split"));
    }
}
