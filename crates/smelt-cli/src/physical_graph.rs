use crate::compiler::{resolve_refs_in_sql, CompilerRegistry, EphemeralResolver};
use crate::discovery::{ModelFile, ModelKind};
use crate::logical_graph::LogicalGraph;
use crate::transformer::TimeRange;
use anyhow::{anyhow, Result};
use smelt_core::config::{IncrementalConfig, Materialization};
use smelt_core::{ModelId, RefInfo};
use smelt_planner::{ExecutionStep, Transformation};
use std::collections::{HashMap, HashSet};

/// How a physical node should be executed.
#[derive(Debug, Clone, serde::Serialize)]
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
#[derive(Debug)]
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
    /// Which user-authored model(s) this node traces back to.
    /// For user-authored models, contains just the model's own name.
    /// For synthetic nodes, contains the origin model name.
    pub logical_origins: Vec<String>,
}

/// The physical execution graph — what the executor consumes.
///
/// Contains only executable nodes (no ephemerals) in topological order,
/// with pre-built ephemeral resolvers per target.
#[derive(Debug)]
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

    /// Filter physical nodes to those whose logical origins intersect with the given selection.
    ///
    /// Used to map `--select` on logical model names to physical execution nodes.
    /// Synthetic intermediates are included if their origin model is selected.
    pub fn filter_for_selection(&self, selected_logical: &HashSet<String>) -> Vec<&str> {
        self.execution_order
            .iter()
            .filter(|name| {
                self.nodes
                    .get(name.as_str())
                    .map(|node| {
                        node.logical_origins
                            .iter()
                            .any(|origin| selected_logical.contains(origin))
                    })
                    .unwrap_or(false)
            })
            .map(|s| s.as_str())
            .collect()
    }
}

/// Apply ref redirects to a model file's SQL content.
///
/// Replaces `smelt.models.from` with `smelt.models.to` for each redirect,
/// and updates the refs list accordingly.
fn apply_ref_redirects(model_file: &ModelFile, redirects: &HashMap<String, String>) -> ModelFile {
    let mut content = model_file.content.clone();
    let mut refs = model_file.refs.clone();

    for (from, to) in redirects {
        // Replace both single-quoted and double-quoted ref calls
        content = content.replace(
            &format!("smelt.models.{}", from),
            &format!("smelt.models.{}", to),
        );
        content = content.replace(
            &format!("smelt.ref(\"{}\")", from),
            &format!("smelt.ref(\"{}\")", to),
        );

        // Update refs list
        for r in &mut refs {
            if r.model_name == *from {
                r.model_name = to.clone();
            }
        }
    }

    ModelFile {
        name: model_file.name.clone(),
        model_id: model_file.model_id.clone(),
        path: model_file.path.clone(),
        content,
        refs,
        parse_errors: model_file.parse_errors.clone(),
        metadata: model_file.metadata.clone(),
        kind: model_file.kind.clone(),
    }
}

/// Builds a PhysicalGraph from a LogicalGraph + planner transformations.
pub struct PhysicalGraphBuilder<'a> {
    logical_graph: &'a LogicalGraph,
    transformations: &'a [Transformation],
    time_range: Option<TimeRange>,
    compilers: Option<&'a CompilerRegistry>,
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
            compilers: Some(compilers),
            target_schemas,
        }
    }

    /// Create a builder for explain mode (no compilers needed).
    ///
    /// Skips ephemeral resolver construction and ref resolution in plan steps,
    /// but still resolves strategies and applies transformations.
    pub fn for_explain(
        logical_graph: &'a LogicalGraph,
        transformations: &'a [Transformation],
        target_schemas: HashMap<String, String>,
    ) -> Self {
        Self {
            logical_graph,
            transformations,
            time_range: None,
            compilers: None,
            target_schemas,
        }
    }

    pub fn build(self) -> Result<PhysicalGraph> {
        // 1. Parse transformations into lookup maps
        let mut plan_overrides: HashMap<String, Vec<ExecutionStep>> = HashMap::new();
        let mut incremental_overrides: HashMap<String, (String, String, smelt_core::Granularity)> =
            HashMap::new();
        let mut synthetic_nodes: Vec<(String, String, Vec<String>, String, Materialization)> =
            Vec::new();
        let mut removed_nodes: HashSet<String> = HashSet::new();
        let mut ref_redirects: HashMap<String, String> = HashMap::new();
        let mut materialization_overrides: HashMap<String, Materialization> = HashMap::new();

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
                Transformation::CreateNode {
                    name,
                    sql,
                    dependencies,
                    origin,
                    materialization,
                } => {
                    synthetic_nodes.push((
                        name.clone(),
                        sql.clone(),
                        dependencies.clone(),
                        origin.clone(),
                        materialization.clone(),
                    ));
                }
                Transformation::RemoveNode { model } => {
                    removed_nodes.insert(model.clone());
                }
                Transformation::RedirectRef { from, to } => {
                    ref_redirects.insert(from.clone(), to.clone());
                }
                Transformation::SetMaterialization {
                    model,
                    materialization,
                } => {
                    materialization_overrides.insert(model.clone(), materialization.clone());
                }
            }
        }

        // 2. Validate graph-level transformations
        self.validate_transformations(
            &synthetic_nodes,
            &removed_nodes,
            &ref_redirects,
            &materialization_overrides,
        )?;

        // 3. Build ephemeral resolvers per target
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
        if let Some(compilers) = self.compilers {
            for (target_name, models) in &ephemeral_models_by_target {
                let compiler = compilers.get(target_name);
                let schema = &self.target_schemas[target_name];
                ephemeral_resolvers.insert(
                    target_name.clone(),
                    compiler.build_ephemeral_resolver(models, schema),
                );
            }
        }

        // 4. Build physical nodes for non-ephemeral, non-removed models
        let mut nodes = HashMap::new();
        let mut physical_order = Vec::new();

        for model_name in &execution_order {
            // Skip removed models
            if removed_nodes.contains(model_name.as_str()) {
                continue;
            }

            let node = self.logical_graph.get_node(model_name)?;

            // Skip ephemeral models — they are inlined as CTEs
            if node.materialization == Materialization::Ephemeral {
                continue;
            }

            // Apply materialization override if present
            let materialization = materialization_overrides
                .get(model_name.as_str())
                .cloned()
                .unwrap_or_else(|| node.materialization.clone());

            // Apply ref redirects to model SQL content
            let model_file = if ref_redirects.is_empty() {
                node.model_file.clone()
            } else {
                apply_ref_redirects(&node.model_file, &ref_redirects)
            };

            let schema = self.target_schemas.get(&node.target).map(|s| s.as_str());

            // Resolve plan steps with ref resolution (skipped in explain mode)
            let plan_steps = if self.compilers.is_some() {
                self.resolve_plan_steps(model_name, &plan_overrides, schema.unwrap_or("main"))
            } else {
                // In explain mode, keep plan steps as-is (no ref resolution)
                plan_overrides.get(model_name).cloned()
            };

            // Resolve execution strategy
            let strategy = self.resolve_strategy(
                model_name,
                &incremental_overrides,
                node.incremental.as_ref(),
                plan_steps,
            );

            physical_order.push(model_name.clone());
            nodes.insert(
                model_name.clone(),
                PhysicalNode {
                    name: model_name.clone(),
                    model_file,
                    materialization,
                    target: node.target.clone(),
                    strategy,
                    logical_origins: vec![model_name.clone()],
                },
            );
        }

        // 5. Add synthetic nodes from CreateNode transformations
        for (name, sql, dependencies, origin, materialization) in &synthetic_nodes {
            // Synthetic nodes inherit the target of their origin model
            let origin_target = if let Ok(origin_node) = self.logical_graph.get_node(origin) {
                origin_node.target.clone()
            } else {
                // If origin doesn't exist in logical graph, use first available target
                self.target_schemas
                    .keys()
                    .next()
                    .cloned()
                    .unwrap_or_default()
            };

            let path: std::path::PathBuf = format!("{}.sql", name).into();
            let model_file = ModelFile {
                name: name.clone(),
                model_id: ModelId::from_path(path.clone()),
                path,
                content: sql.clone(),
                refs: dependencies
                    .iter()
                    .map(|dep| RefInfo {
                        model_name: dep.clone(),
                        has_named_params: false,
                        range: rowan::TextRange::default(),
                        smelt_ref: smelt_core::SmeltRef::Path(vec![
                            "models".to_string(),
                            dep.clone(),
                        ]),
                    })
                    .collect(),
                parse_errors: Vec::new(),
                metadata: None,
                kind: ModelKind::Sql,
            };

            // Insert synthetic node into execution order respecting dependencies.
            // It must come after all its dependencies.
            let insert_pos = dependencies
                .iter()
                .filter_map(|dep| physical_order.iter().position(|n| n == dep))
                .max()
                .map(|pos| pos + 1)
                .unwrap_or(physical_order.len());

            physical_order.insert(insert_pos, name.clone());
            nodes.insert(
                name.clone(),
                PhysicalNode {
                    name: name.clone(),
                    model_file,
                    materialization: materialization.clone(),
                    target: origin_target,
                    strategy: PhysicalStrategy::FullRefresh,
                    logical_origins: vec![origin.clone()],
                },
            );
        }

        Ok(PhysicalGraph {
            nodes,
            execution_order: physical_order,
            ephemeral_resolvers,
        })
    }

    /// Resolve plan steps for a model, applying ref resolution to SQL.
    fn resolve_plan_steps(
        &self,
        model_name: &str,
        plan_overrides: &HashMap<String, Vec<ExecutionStep>>,
        schema: &str,
    ) -> Option<Vec<ExecutionStep>> {
        plan_overrides.get(model_name).map(|steps| {
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
        })
    }

    /// Resolve the execution strategy for a model.
    fn resolve_strategy(
        &self,
        model_name: &str,
        incremental_overrides: &HashMap<String, (String, String, smelt_core::Granularity)>,
        config_incremental: Option<&IncrementalConfig>,
        plan_steps: Option<Vec<ExecutionStep>>,
    ) -> PhysicalStrategy {
        if let Some((event_time_col, partition_col, granularity)) =
            incremental_overrides.get(model_name)
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
        } else if let Some(inc) = config_incremental.filter(|_| self.time_range.is_some()) {
            // Config-based incremental (smelt.yml or frontmatter)
            PhysicalStrategy::Incremental {
                config: inc.clone(),
                time_range: self
                    .time_range
                    .as_ref()
                    .expect("time_range is Some when filter passes")
                    .clone(),
                plan_steps,
            }
        } else if let Some(steps) = plan_steps {
            PhysicalStrategy::CubeSplit { steps }
        } else {
            PhysicalStrategy::FullRefresh
        }
    }

    /// Validate graph-level transformations before applying them.
    fn validate_transformations(
        &self,
        synthetic_nodes: &[(String, String, Vec<String>, String, Materialization)],
        removed_nodes: &HashSet<String>,
        ref_redirects: &HashMap<String, String>,
        materialization_overrides: &HashMap<String, Materialization>,
    ) -> Result<()> {
        let logical_names: HashSet<&str> = self.logical_graph.models().keys().copied().collect();
        let synthetic_names: HashSet<&str> =
            synthetic_nodes.iter().map(|(n, ..)| n.as_str()).collect();

        // Validate RemoveNode: model must exist in logical graph
        for name in removed_nodes {
            if !logical_names.contains(name.as_str()) {
                return Err(anyhow!(
                    "RemoveNode transformation references unknown model '{}'",
                    name
                ));
            }
        }

        // Validate RedirectRef: 'from' must exist, 'to' must exist (in logical or synthetic)
        for (from, to) in ref_redirects {
            if !logical_names.contains(from.as_str()) {
                return Err(anyhow!(
                    "RedirectRef transformation: source model '{}' does not exist",
                    from
                ));
            }
            if !logical_names.contains(to.as_str()) && !synthetic_names.contains(to.as_str()) {
                return Err(anyhow!(
                    "RedirectRef transformation: target model '{}' does not exist",
                    to
                ));
            }
        }

        // Validate SetMaterialization: model must exist
        for name in materialization_overrides.keys() {
            if !logical_names.contains(name.as_str()) {
                return Err(anyhow!(
                    "SetMaterialization transformation references unknown model '{}'",
                    name
                ));
            }
        }

        // Validate CreateNode: dependencies must exist (in logical graph or other synthetic nodes)
        for (name, _, deps, _, _) in synthetic_nodes {
            if logical_names.contains(name.as_str()) {
                return Err(anyhow!(
                    "CreateNode '{}' conflicts with existing model name",
                    name
                ));
            }
            for dep in deps {
                if !logical_names.contains(dep.as_str()) && !synthetic_names.contains(dep.as_str())
                {
                    return Err(anyhow!(
                        "CreateNode '{}' depends on unknown model '{}'",
                        name,
                        dep
                    ));
                }
            }
        }

        Ok(())
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
                smelt_ref: smelt_core::refs::SmeltRef::Path(vec![
                    "models".to_string(),
                    dep.to_string(),
                ]),
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
            models: HashMap::new(),
            python: None,
        }
    }

    fn build_graph_and_compilers(
        models: Vec<ModelFile>,
        config: &Config,
    ) -> (LogicalGraph, CompilerRegistry, HashMap<String, String>) {
        let graph = LogicalGraph::build(models, None, &[], config, "dev").unwrap();
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

    // --- Phase C: Graph-level transformation tests ---

    #[test]
    fn test_create_node_adds_synthetic() {
        let models = vec![make_model("A", vec![]), make_model("B", vec!["A"])];
        let config = make_test_config();

        let transformations = vec![Transformation::CreateNode {
            name: "__smelt__A__shared__abc12345".to_string(),
            sql: "SELECT * FROM A".to_string(),
            dependencies: vec!["A".to_string()],
            origin: "A".to_string(),
            materialization: Materialization::Table,
        }];

        let (graph, compilers, schemas) = build_graph_and_compilers(models, &config);
        let pg = PhysicalGraphBuilder::new(&graph, &transformations, None, &compilers, schemas)
            .build()
            .unwrap();

        // Synthetic node should exist
        assert_eq!(pg.execution_order().len(), 3);
        let synthetic = pg.get_node("__smelt__A__shared__abc12345").unwrap();
        assert_eq!(synthetic.materialization, Materialization::Table);
        assert_eq!(synthetic.logical_origins, vec!["A"]);

        // Synthetic node should come after its dependency A
        let a_pos = pg.execution_order().iter().position(|n| n == "A").unwrap();
        let syn_pos = pg
            .execution_order()
            .iter()
            .position(|n| n == "__smelt__A__shared__abc12345")
            .unwrap();
        assert!(syn_pos > a_pos);
    }

    #[test]
    fn test_remove_node_excludes_model() {
        let models = vec![make_model("A", vec![]), make_model("B", vec![])];
        let config = make_test_config();

        let transformations = vec![Transformation::RemoveNode {
            model: "A".to_string(),
        }];

        let (graph, compilers, schemas) = build_graph_and_compilers(models, &config);
        let pg = PhysicalGraphBuilder::new(&graph, &transformations, None, &compilers, schemas)
            .build()
            .unwrap();

        assert_eq!(pg.execution_order().len(), 1);
        assert_eq!(pg.execution_order()[0], "B");
        assert!(pg.get_node("A").is_none());
    }

    #[test]
    fn test_redirect_ref_rewrites_sql() {
        let config = make_test_config();

        // Create a model with a ref to "source" in its content
        let mut model_with_ref = make_model("downstream", vec!["source"]);
        model_with_ref.content = "SELECT * FROM smelt.models.source WHERE id > 0".to_string();

        let all_models = vec![
            make_model("source", vec![]),
            make_model("target", vec![]),
            model_with_ref,
        ];

        let transformations = vec![Transformation::RedirectRef {
            from: "source".to_string(),
            to: "target".to_string(),
        }];

        let (graph, compilers, schemas) = build_graph_and_compilers(all_models, &config);
        let pg = PhysicalGraphBuilder::new(&graph, &transformations, None, &compilers, schemas)
            .build()
            .unwrap();

        let node = pg.get_node("downstream").unwrap();
        assert!(node.model_file.content.contains("smelt.models.target"));
        assert!(!node.model_file.content.contains("smelt.models.source"));
    }

    #[test]
    fn test_set_materialization_overrides() {
        let models = vec![make_model("A", vec![])];
        let config = make_test_config();
        // Default materialization is View (from config)

        let transformations = vec![Transformation::SetMaterialization {
            model: "A".to_string(),
            materialization: Materialization::Table,
        }];

        let (graph, compilers, schemas) = build_graph_and_compilers(models, &config);
        let pg = PhysicalGraphBuilder::new(&graph, &transformations, None, &compilers, schemas)
            .build()
            .unwrap();

        let node = pg.get_node("A").unwrap();
        assert_eq!(node.materialization, Materialization::Table);
    }

    #[test]
    fn test_logical_origins_for_user_models() {
        let models = vec![make_model("A", vec![]), make_model("B", vec!["A"])];
        let config = make_test_config();

        let (graph, compilers, schemas) = build_graph_and_compilers(models, &config);
        let pg = PhysicalGraphBuilder::new(&graph, &[], None, &compilers, schemas)
            .build()
            .unwrap();

        assert_eq!(pg.get_node("A").unwrap().logical_origins, vec!["A"]);
        assert_eq!(pg.get_node("B").unwrap().logical_origins, vec!["B"]);
    }

    #[test]
    fn test_filter_for_selection() {
        let models = vec![make_model("A", vec![]), make_model("B", vec!["A"])];
        let config = make_test_config();

        let transformations = vec![Transformation::CreateNode {
            name: "__smelt__A__tmp__12345678".to_string(),
            sql: "SELECT 1".to_string(),
            dependencies: vec!["A".to_string()],
            origin: "A".to_string(),
            materialization: Materialization::Table,
        }];

        let (graph, compilers, schemas) = build_graph_and_compilers(models, &config);
        let pg = PhysicalGraphBuilder::new(&graph, &transformations, None, &compilers, schemas)
            .build()
            .unwrap();

        // Selecting "A" should include A and its synthetic node
        let selected: HashSet<String> = ["A".to_string()].into_iter().collect();
        let filtered = pg.filter_for_selection(&selected);
        assert!(filtered.contains(&"A"));
        assert!(filtered.contains(&"__smelt__A__tmp__12345678"));
        assert!(!filtered.contains(&"B"));
    }

    // --- Validation error tests ---

    #[test]
    fn test_create_node_duplicate_name_error() {
        let models = vec![make_model("A", vec![])];
        let config = make_test_config();

        let transformations = vec![Transformation::CreateNode {
            name: "A".to_string(), // conflicts with existing
            sql: "SELECT 1".to_string(),
            dependencies: vec![],
            origin: "A".to_string(),
            materialization: Materialization::Table,
        }];

        let (graph, compilers, schemas) = build_graph_and_compilers(models, &config);
        let result =
            PhysicalGraphBuilder::new(&graph, &transformations, None, &compilers, schemas).build();

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("conflicts with existing"));
    }

    #[test]
    fn test_create_node_unknown_dep_error() {
        let models = vec![make_model("A", vec![])];
        let config = make_test_config();

        let transformations = vec![Transformation::CreateNode {
            name: "__smelt__synth".to_string(),
            sql: "SELECT 1".to_string(),
            dependencies: vec!["nonexistent".to_string()],
            origin: "A".to_string(),
            materialization: Materialization::Table,
        }];

        let (graph, compilers, schemas) = build_graph_and_compilers(models, &config);
        let result =
            PhysicalGraphBuilder::new(&graph, &transformations, None, &compilers, schemas).build();

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown model"));
    }

    #[test]
    fn test_remove_node_unknown_model_error() {
        let models = vec![make_model("A", vec![])];
        let config = make_test_config();

        let transformations = vec![Transformation::RemoveNode {
            model: "nonexistent".to_string(),
        }];

        let (graph, compilers, schemas) = build_graph_and_compilers(models, &config);
        let result =
            PhysicalGraphBuilder::new(&graph, &transformations, None, &compilers, schemas).build();

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown model"));
    }

    #[test]
    fn test_set_materialization_unknown_model_error() {
        let models = vec![make_model("A", vec![])];
        let config = make_test_config();

        let transformations = vec![Transformation::SetMaterialization {
            model: "nonexistent".to_string(),
            materialization: Materialization::Table,
        }];

        let (graph, compilers, schemas) = build_graph_and_compilers(models, &config);
        let result =
            PhysicalGraphBuilder::new(&graph, &transformations, None, &compilers, schemas).build();

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown model"));
    }
}
