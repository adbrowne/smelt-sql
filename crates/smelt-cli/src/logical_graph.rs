use crate::config::Config;
use crate::discovery::ModelFile;
use anyhow::{anyhow, Result};
use smelt_core::config::{IncrementalConfig, Materialization};
use smelt_core::selector::{SelectionMethod, Selector};
use smelt_core::{GraphError, SeedInfo, SourcesConfig};
use std::collections::{HashMap, HashSet, VecDeque};

/// A node in the logical graph with eagerly-resolved configuration.
pub struct LogicalNode {
    pub name: String,
    pub model_file: ModelFile,
    pub dependencies: Vec<String>,
    pub materialization: Materialization,
    pub incremental: Option<IncrementalConfig>,
    pub target: String,
    pub tags: Vec<String>,
}

/// Describes a dependency edge that crosses backend boundaries.
pub struct CrossBackendEdge {
    pub downstream: String,
    pub upstream: String,
    pub upstream_target: String,
    pub downstream_target: String,
}

/// A dependency graph where each node carries its fully-resolved configuration.
///
/// Unlike `DependencyGraph`, the config cascade (SQL metadata > smelt.yml > default) is
/// resolved at construction time, so consumers don't need to carry `Config` around.
pub struct LogicalGraph {
    nodes: HashMap<String, LogicalNode>,
    sources: HashSet<String>,
    /// Top-level seed names (CSV files in `seeds/`). Treated as valid
    /// `smelt.ref()` targets — they have no upstream node, just exist as
    /// "sources" populated by `smelt seed`.
    seeds: HashSet<String>,
}

impl LogicalGraph {
    /// Build a logical graph from discovered models, resolving config eagerly.
    ///
    /// `seeds` is the list of top-level seed CSVs discovered via
    /// `smelt_core::discover_seed_infos`. Their names become valid
    /// `smelt.ref()` targets (no upstream node — they're populated by
    /// `smelt seed` and treated like sources during validation).
    pub fn build(
        models: Vec<ModelFile>,
        sources: Option<&SourcesConfig>,
        seeds: &[SeedInfo],
        config: &Config,
        default_target: &str,
    ) -> Result<Self> {
        let mut nodes = HashMap::new();

        // Build source set (schema.table format)
        let mut source_set = HashSet::new();
        if let Some(sources) = sources {
            for source in &sources.sources {
                for table in &source.tables {
                    source_set.insert(format!("{}.{}", source.name, table.name));
                }
            }
        }

        // Build seed name set so `smelt.<seed>` validates without a
        // sources.yml workaround. Mirrors `smelt-db::resolve_ref` (Phase 6),
        // which the CLI dependency validator was previously missing.
        let seed_set: HashSet<String> = seeds.iter().map(|s| s.name.clone()).collect();

        for model in models {
            // Extract model-to-model deps from Path-form refs.
            //
            // Phase 2 (unified-paths): after scan-root stripping, model refs
            // have the form `smelt.<leaf>` or `smelt.<dir>.<leaf>` — no
            // "models" prefix. Exclude only known non-model namespaces:
            //   - "sources" (smelt.sources.schema.table)
            //   - "functions" (smelt.functions.fn_name(...))
            //
            // All other paths are potential model/seed deps; `validate()` will
            // flag any that don't appear in nodes, sources, or seeds.
            let deps: Vec<String> = model
                .refs
                .iter()
                .filter_map(|r| {
                    let path = r.smelt_ref.to_path();
                    let first = path.first().map(|s| s.as_str());
                    // Skip well-known non-model namespaces.
                    if matches!(first, Some("sources") | Some("functions")) {
                        return None;
                    }
                    path.last().cloned()
                })
                .collect();
            let metadata = model.metadata.as_ref().map(|b| b.as_ref());

            let materialization = config.get_materialization_with_metadata(&model.name, metadata);
            let target = config.get_target(&model.name, metadata, default_target);
            let incremental = config
                .get_incremental_with_metadata(&model.name, metadata)
                .cloned();
            let tags = config.get_tags(&model.name, metadata);

            if let Some(existing) = nodes.get(&model.name) {
                let existing: &LogicalNode = existing;
                tracing::warn!(
                    "Duplicate model name '{}'. Model at {} overwrites model at {}.",
                    model.name,
                    model.path.display(),
                    existing.model_file.path.display()
                );
            }

            nodes.insert(
                model.name.clone(),
                LogicalNode {
                    name: model.name.clone(),
                    dependencies: deps,
                    materialization,
                    incremental,
                    target,
                    tags,
                    model_file: model,
                },
            );
        }

        Ok(Self {
            nodes,
            sources: source_set,
            seeds: seed_set,
        })
    }

    // -- Validation ----------------------------------------------------------

    /// Validate all references exist (as models, sources, or seeds).
    pub fn validate(&self) -> Result<()> {
        let mut errors = Vec::new();

        for node in self.nodes.values() {
            for dep in &node.dependencies {
                if !self.nodes.contains_key(dep) && !self.is_source(dep) && !self.is_seed(dep) {
                    errors.push(format!(
                        "Model '{}' references undefined model/source '{}'",
                        node.name, dep
                    ));
                }
            }
        }

        if !errors.is_empty() {
            return Err(GraphError::DependencyError {
                message: errors.join("\n  "),
            }
            .into());
        }

        Ok(())
    }

    /// Find all dependency edges that cross backend boundaries.
    pub fn find_cross_backend_edges(&self) -> Vec<CrossBackendEdge> {
        let mut edges = Vec::new();

        for node in self.nodes.values() {
            for dep in &node.dependencies {
                if let Some(dep_node) = self.nodes.get(dep) {
                    if node.target != dep_node.target {
                        edges.push(CrossBackendEdge {
                            downstream: node.name.clone(),
                            upstream: dep.clone(),
                            upstream_target: dep_node.target.clone(),
                            downstream_target: node.target.clone(),
                        });
                    }
                }
            }
        }

        edges
    }

    fn is_source(&self, name: &str) -> bool {
        self.sources.contains(name)
            || self
                .sources
                .iter()
                .any(|s| s.ends_with(&format!(".{}", name)))
    }

    fn is_seed(&self, name: &str) -> bool {
        self.seeds.contains(name)
    }

    /// Iterate seed names that are valid `smelt.ref()` targets.
    pub fn iter_seeds(&self) -> impl Iterator<Item = &str> {
        self.seeds.iter().map(|s| s.as_str())
    }

    /// Warn if any ephemeral model has no downstream consumers.
    pub fn warn_unused_ephemerals(&self) {
        let dependents = self.build_dependents_map();
        for node in self.nodes.values() {
            if node.materialization == Materialization::Ephemeral {
                let has_consumers = dependents.get(&node.name).is_some_and(|d| !d.is_empty());
                if !has_consumers {
                    tracing::warn!(
                        "Ephemeral model '{}' has no downstream consumers and will never be inlined.",
                        node.name
                    );
                }
            }
        }
    }

    // -- Topological sort ----------------------------------------------------

    /// Topological sort to determine execution order using Kahn's algorithm.
    pub fn execution_order(&self) -> Result<Vec<String>> {
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut dependents: HashMap<String, Vec<String>> = HashMap::new();

        for name in self.nodes.keys() {
            in_degree.insert(name.clone(), 0);
            dependents.insert(name.clone(), Vec::new());
        }

        for node in self.nodes.values() {
            for dep in &node.dependencies {
                if self.nodes.contains_key(dep) {
                    *in_degree
                        .get_mut(&node.name)
                        .expect("all node names were inserted into in_degree") += 1;
                    dependents
                        .get_mut(dep)
                        .expect("all node names were inserted into dependents")
                        .push(node.name.clone());
                }
            }
        }

        let mut queue: VecDeque<String> = in_degree
            .iter()
            .filter(|(_, &degree)| degree == 0)
            .map(|(name, _)| name.clone())
            .collect();

        let mut order = Vec::new();

        while let Some(model_name) = queue.pop_front() {
            order.push(model_name.clone());

            if let Some(deps) = dependents.get(&model_name) {
                for dependent in deps {
                    let degree = in_degree
                        .get_mut(dependent)
                        .expect("dependents only contains valid node names");
                    *degree -= 1;

                    if *degree == 0 {
                        queue.push_back(dependent.clone());
                    }
                }
            }
        }

        if order.len() != self.nodes.len() {
            let remaining: Vec<_> = in_degree
                .iter()
                .filter(|(_, &degree)| degree > 0)
                .map(|(name, _)| name.as_str())
                .collect();

            return Err(GraphError::CircularDependency {
                models: remaining.join(", "),
            }
            .into());
        }

        Ok(order)
    }

    /// Filter execution order to only include selected models.
    pub fn filtered_execution_order(&self, selected: &HashSet<String>) -> Result<Vec<String>> {
        let full_order = self.execution_order()?;
        Ok(full_order
            .into_iter()
            .filter(|name| selected.contains(name))
            .collect())
    }

    // -- Model selection -----------------------------------------------------

    /// Select models matching the given selectors, with optional upstream/downstream expansion.
    ///
    /// Tags are read from the pre-resolved `LogicalNode.tags` (no Config needed).
    pub fn select_models(&self, selectors: &[Selector]) -> Result<HashSet<String>> {
        let mut selected = HashSet::new();
        let dependents = self.build_dependents_map();

        for selector in selectors {
            let direct_matches: Vec<String> = match &selector.method {
                SelectionMethod::ModelName(name) => {
                    if self.nodes.contains_key(name) {
                        vec![name.clone()]
                    } else {
                        return Err(anyhow!("Model '{}' not found", name));
                    }
                }
                SelectionMethod::Tag(tag) => self
                    .nodes
                    .values()
                    .filter(|node| node.tags.contains(tag))
                    .map(|node| node.name.clone())
                    .collect(),
            };

            for model_name in &direct_matches {
                selected.insert(model_name.clone());
            }

            if selector.include_upstream {
                for model_name in &direct_matches {
                    self.collect_upstream(model_name, &mut selected);
                }
            }

            if selector.include_downstream {
                for model_name in &direct_matches {
                    self.collect_downstream(model_name, &dependents, &mut selected);
                }
            }
        }

        Ok(selected)
    }

    /// Remove models matching the given exclude selectors from the selected set.
    pub fn exclude_models(
        &self,
        selected: &HashSet<String>,
        excludes: &[Selector],
    ) -> Result<HashSet<String>> {
        let to_exclude = self.select_models(excludes)?;
        Ok(selected.difference(&to_exclude).cloned().collect())
    }

    // -- Accessors -----------------------------------------------------------

    pub fn get_model(&self, name: &str) -> Result<&ModelFile> {
        self.nodes
            .get(name)
            .map(|n| &n.model_file)
            .ok_or_else(|| anyhow!("Model not found: {}", name))
    }

    pub fn get_node(&self, name: &str) -> Result<&LogicalNode> {
        self.nodes
            .get(name)
            .ok_or_else(|| anyhow!("Model not found: {}", name))
    }

    pub fn model_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn all_model_names(&self) -> HashSet<String> {
        self.nodes.keys().cloned().collect()
    }

    pub fn iter_models(&self) -> impl Iterator<Item = (&str, &ModelFile)> {
        self.nodes.iter().map(|(k, v)| (k.as_str(), &v.model_file))
    }

    pub fn iter_nodes(&self) -> impl Iterator<Item = &LogicalNode> {
        self.nodes.values()
    }

    pub fn iter_dependencies(&self) -> impl Iterator<Item = (&str, &[String])> {
        self.nodes
            .iter()
            .map(|(k, v)| (k.as_str(), v.dependencies.as_slice()))
    }

    pub fn iter_sources(&self) -> impl Iterator<Item = &str> {
        self.sources.iter().map(|s| s.as_str())
    }

    /// Get a view of all models (for backward compatibility with code expecting HashMap).
    pub fn models(&self) -> HashMap<&str, &ModelFile> {
        self.nodes
            .iter()
            .map(|(k, v)| (k.as_str(), &v.model_file))
            .collect()
    }

    /// Get the upstream dependencies for a model (model names it references).
    pub fn get_upstream(&self, model_name: &str) -> Vec<String> {
        self.nodes
            .get(model_name)
            .map(|node| {
                node.dependencies
                    .iter()
                    .filter(|dep| self.nodes.contains_key(dep.as_str()))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Collect all upstream dependencies recursively.
    pub fn all_upstream(&self, model_name: &str) -> HashSet<String> {
        let mut result = HashSet::new();
        self.collect_upstream(model_name, &mut result);
        result
    }

    // -- Internal helpers ----------------------------------------------------

    fn collect_upstream(&self, model_name: &str, result: &mut HashSet<String>) {
        if let Some(node) = self.nodes.get(model_name) {
            for dep in &node.dependencies {
                if self.nodes.contains_key(dep) && result.insert(dep.clone()) {
                    self.collect_upstream(dep, result);
                }
            }
        }
    }

    fn build_dependents_map(&self) -> HashMap<String, Vec<String>> {
        let mut dependents: HashMap<String, Vec<String>> = HashMap::new();
        for node in self.nodes.values() {
            for dep in &node.dependencies {
                if self.nodes.contains_key(dep) {
                    dependents
                        .entry(dep.clone())
                        .or_default()
                        .push(node.name.clone());
                }
            }
        }
        dependents
    }

    fn collect_downstream(
        &self,
        model_name: &str,
        dependents: &HashMap<String, Vec<String>>,
        result: &mut HashSet<String>,
    ) {
        if let Some(deps) = dependents.get(model_name) {
            for dep in deps {
                if result.insert(dep.clone()) {
                    self.collect_downstream(dep, dependents, result);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::ModelKind;
    use rowan::TextRange;
    use smelt_core::config::{ModelConfig, Target};
    use smelt_core::ModelId;
    use smelt_core::RefInfo;

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
            content: String::new(),
            refs,
            parse_errors: Vec::new(),
            metadata: None,
            kind: ModelKind::Sql,
            address_segments: vec![name.to_string()],
        }
    }

    fn make_model_with_tags(name: &str, deps: Vec<&str>, tags: Vec<&str>) -> ModelFile {
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

        let metadata = if tags.is_empty() {
            None
        } else {
            Some(Box::new(crate::metadata::ModelMetadata {
                tags: tags.into_iter().map(|t| t.to_string()).collect(),
                ..Default::default()
            }))
        };

        let path: std::path::PathBuf = format!("{}.sql", name).into();
        ModelFile {
            name: name.to_string(),
            model_id: ModelId::from_path(path.clone()),
            path,
            content: String::new(),
            refs,
            parse_errors: Vec::new(),
            metadata,
            kind: ModelKind::Sql,
            address_segments: vec![name.to_string()],
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
            paths: vec!["models".to_string()],
            targets,
            default_materialization: Materialization::View,
            models: HashMap::new(),
            python: None,
        }
    }

    #[test]
    fn test_linear_dependency() {
        let models = vec![
            make_model("C", vec!["B"]),
            make_model("B", vec!["A"]),
            make_model("A", vec![]),
        ];
        let config = make_test_config();
        let graph = LogicalGraph::build(models, None, &[], &config, "dev").unwrap();
        graph.validate().unwrap();

        let order = graph.execution_order().unwrap();
        assert_eq!(order, vec!["A", "B", "C"]);
    }

    #[test]
    fn test_eager_config_resolution() {
        let models = vec![make_model("A", vec![])];
        let mut config = make_test_config();
        config.models.insert(
            "A".to_string(),
            ModelConfig {
                materialization: Some(Materialization::Table),
                incremental: None,
                tags: vec!["core".to_string()],
                target: None,
            },
        );

        let graph = LogicalGraph::build(models, None, &[], &config, "dev").unwrap();
        let node = graph.get_node("A").unwrap();

        assert_eq!(node.materialization, Materialization::Table);
        assert_eq!(node.target, "dev");
        assert_eq!(node.tags, vec!["core".to_string()]);
        assert!(node.incremental.is_none());
    }

    #[test]
    fn test_select_by_tag_no_config() {
        let models = vec![
            make_model_with_tags("A", vec![], vec!["core"]),
            make_model_with_tags("B", vec!["A"], vec!["revenue"]),
            make_model_with_tags("C", vec!["B"], vec!["revenue", "daily"]),
        ];
        let config = make_test_config();
        let graph = LogicalGraph::build(models, None, &[], &config, "dev").unwrap();

        let selectors = vec![smelt_core::parse_selector("tag:revenue").unwrap()];
        let selected = graph.select_models(&selectors).unwrap();

        assert_eq!(selected.len(), 2);
        assert!(selected.contains("B"));
        assert!(selected.contains("C"));
    }

    #[test]
    fn test_cross_backend_edges_detected() {
        let models = vec![
            make_model("upstream", vec![]),
            make_model("downstream", vec!["upstream"]),
        ];
        let mut config = make_test_config();
        config.targets.insert(
            "spark_prod".to_string(),
            Target {
                target_type: "spark".to_string(),
                database: None,
                schema: "default".to_string(),
                connect_url: None,
                catalog: None,
                warehouse: None,
                format: None,
            },
        );
        config.models.insert(
            "upstream".to_string(),
            ModelConfig {
                materialization: None,
                incremental: None,
                tags: vec![],
                target: Some("spark_prod".to_string()),
            },
        );

        let graph = LogicalGraph::build(models, None, &[], &config, "dev").unwrap();
        let edges = graph.find_cross_backend_edges();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].upstream, "upstream");
        assert_eq!(edges[0].downstream, "downstream");
        assert_eq!(edges[0].upstream_target, "spark_prod");
        assert_eq!(edges[0].downstream_target, "dev");
    }

    #[test]
    fn test_select_upstream() {
        let models = vec![
            make_model("A", vec![]),
            make_model("B", vec!["A"]),
            make_model("C", vec!["B"]),
        ];
        let config = make_test_config();
        let graph = LogicalGraph::build(models, None, &[], &config, "dev").unwrap();

        let selectors = vec![smelt_core::parse_selector("+C").unwrap()];
        let selected = graph.select_models(&selectors).unwrap();

        assert_eq!(selected.len(), 3);
        assert!(selected.contains("A"));
        assert!(selected.contains("B"));
        assert!(selected.contains("C"));
    }

    #[test]
    fn test_circular_dependency() {
        let models = vec![
            make_model("A", vec!["C"]),
            make_model("B", vec!["A"]),
            make_model("C", vec!["B"]),
        ];
        let config = make_test_config();
        let graph = LogicalGraph::build(models, None, &[], &config, "dev").unwrap();
        let result = graph.execution_order();
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Circular dependency"));
    }

    fn make_seed(name: &str) -> smelt_core::SeedInfo {
        smelt_core::SeedInfo {
            name: name.to_string(),
            path: format!("seeds/{}.csv", name).into(),
            columns: Vec::new(),
            address_segments: vec![name.to_string()],
            sidecar: None,
        }
    }

    /// Bug #2 regression: top-level seeds referenced via `smelt.ref()` must
    /// validate without a sources.yml workaround. Mirrors the e2e idempotency
    /// test (`smelt_shop_idempotency.rs`) but at unit-test speed so the
    /// resolution path can be iterated quickly.
    #[test]
    fn test_seed_as_ref_target_validates() {
        let models = vec![make_model("stg_orders", vec!["order_statuses"])];
        let seeds = vec![make_seed("order_statuses")];
        let config = make_test_config();

        let graph = LogicalGraph::build(models, None, &seeds, &config, "dev").expect("build graph");

        // Validation must succeed: the seed is a valid ref target.
        graph.validate().expect("seed ref should validate");

        // The seed should appear in `iter_seeds()` so `smelt explain` can
        // surface it as a graph node.
        let seed_names: HashSet<&str> = graph.iter_seeds().collect();
        assert!(
            seed_names.contains("order_statuses"),
            "seed should be visible via iter_seeds(): saw {:?}",
            seed_names
        );

        // The model must NOT be treated as depending on a model node — only
        // model-to-model deps participate in the topological execution
        // order. The seed is materialised by `smelt seed`, not by the
        // model graph.
        let upstream = graph.get_upstream("stg_orders");
        assert!(
            upstream.is_empty(),
            "seed ref should not appear as an upstream model node: saw {:?}",
            upstream
        );
    }

    /// A reference that matches neither a model, a source, nor a seed must
    /// still fail validation (so we don't accidentally swallow real typos).
    #[test]
    fn test_unknown_ref_still_fails_validation() {
        let models = vec![make_model("stg_orders", vec!["does_not_exist"])];
        let seeds = vec![make_seed("order_statuses")];
        let config = make_test_config();

        let graph = LogicalGraph::build(models, None, &seeds, &config, "dev").expect("build graph");

        let err = graph
            .validate()
            .expect_err("unknown ref must fail validation");
        assert!(
            err.to_string().contains("does_not_exist"),
            "validation error should mention the missing ref name: {err}"
        );
    }
}
