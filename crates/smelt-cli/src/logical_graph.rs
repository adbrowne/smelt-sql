use crate::config::Config;
use crate::discovery::ModelFile;
use anyhow::{anyhow, Result};
use smelt_core::config::{IncrementalConfig, Materialization, TimeseriesConfig};
use smelt_core::selector::{SelectionMethod, Selector};
use smelt_core::{GraphError, SeedInfo, SourcesConfig};
use std::collections::{HashMap, HashSet, VecDeque};

/// A node in the logical graph with eagerly-resolved configuration.
pub struct LogicalNode {
    pub name: String,
    pub model_file: ModelFile,
    pub dependencies: Vec<String>,
    pub materialization: Materialization,
    pub timeseries: Option<TimeseriesConfig>,
    pub incremental: Option<IncrementalConfig>,
    pub target: String,
    pub tags: Vec<String>,
    /// For generator-emitted models: the workspace-relative path (with `/`
    /// separators) of the generator `.sql` file that produced this model.
    /// `None` for hand-authored models.
    pub generator_file: Option<String>,
    /// For generator-emitted models: the `ModelDef.name` value that produced
    /// this emitted model. `None` for hand-authored models.
    pub generator_name: Option<String>,
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

        // Build the seed address set so `smelt.<path>` validates without a
        // sources.yml workaround. Mirrors `smelt-db::resolve_ref`, which the
        // CLI dependency validator was previously missing. Seeds are keyed by
        // their canonical dot-path (`address_segments.join(".")`), the same key
        // model nodes and dep strings use — a seed at `models/lookup/regions.csv`
        // is addressable as `smelt.lookup.regions`, not just by its leaf name.
        // Keying by leaf `name` alone made sub-directory seeds unresolvable in
        // the CLI run/explain path while the LSP resolver accepted them.
        let seed_set: HashSet<String> = seeds
            .iter()
            .map(|s| {
                let cp = s.address_segments.join(".");
                if cp.is_empty() {
                    s.name.clone()
                } else {
                    cp
                }
            })
            .collect();

        // Guard: detect duplicate canonical addresses before building the graph.
        // Seeds/sources are checked by the Salsa project_address_collisions query;
        // this catches Python-vs-SQL and within-file-section collisions on the
        // CLI build path, where Python models never reach Salsa.
        {
            let (_, collisions) = smelt_core::resolver::resolve_address_map(&models, seeds, &[]);
            if !collisions.is_empty() {
                let msgs: Vec<String> = collisions
                    .iter()
                    .map(|c| {
                        format!(
                            "DuplicateAddress: '{}' is claimed by both {} and {}",
                            c.address.join("."),
                            c.first.path.display(),
                            c.second.path.display(),
                        )
                    })
                    .collect();
                return Err(anyhow!("{}", msgs.join("\n")));
            }
        }

        // Collect unresolved (raw-path) deps alongside each model in a single pass.
        // Deps are resolved to canonical node-name keys in a second pass once all
        // models are in `nodes`.
        struct PendingNode {
            canonical_key: String,
            raw_dep_paths: Vec<Vec<String>>, // each element: one ref's path segments
        }

        let mut pending: Vec<PendingNode> = Vec::with_capacity(models.len());

        // Pass 1: insert all models into `nodes` keyed by canonical_path().
        // canonical_path() == address_segments.join("."), e.g. "silver.events_parsed".
        // For flat models (single segment), canonical_path() == model.name.
        for model in models {
            // Collect raw ref paths (pre-resolution).
            let raw_dep_paths: Vec<Vec<String>> = model
                .refs
                .iter()
                .filter_map(|r| {
                    let path = r.smelt_ref.to_path();
                    let first = path.first().map(|s| s.as_str());
                    // Skip well-known non-model namespaces, including the
                    // meta-language compile-time calls `smelt.config.*` (e.g.
                    // `smelt.config.var`), which the build-path meta evaluator
                    // resolves before codegen and which are not model deps.
                    if matches!(first, Some("sources") | Some("functions") | Some("config")) {
                        return None;
                    }
                    // `smelt.models.with_tag(...)` / `smelt.models.all` are
                    // wide-reflection accessors — compile-time meta the
                    // build-path evaluator resolves before codegen, not model
                    // deps — distinct from a legacy `smelt.models.<leaf>` ref.
                    if first == Some("models")
                        && matches!(
                            path.get(1).map(String::as_str),
                            Some("with_tag") | Some("all")
                        )
                    {
                        return None;
                    }
                    // Strip the legacy "models" namespace prefix (Phase 2 unified
                    // paths eliminated it, but test helpers and some older workspaces
                    // still use `smelt.models.<leaf>` form).
                    let effective: Vec<String> = if first == Some("models") {
                        path[1..].to_vec()
                    } else {
                        path.clone()
                    };
                    if effective.is_empty() {
                        None
                    } else {
                        Some(effective)
                    }
                })
                .collect();

            let metadata = model.metadata.as_ref().map(|b| b.as_ref());

            // Config lookups use model.name (leaf name as declared in smelt.yml models: block).
            let materialization = config.get_materialization_with_metadata(&model.name, metadata);
            let target = config.get_target(&model.name, metadata, default_target);
            let timeseries = config
                .get_timeseries_with_metadata(&model.name, metadata)
                .cloned();
            let incremental = config
                .get_incremental_with_metadata(&model.name, metadata)
                .cloned();
            let tags = config.get_tags(&model.name, metadata);

            // The graph key is the canonical dot-path (address_segments.join(".")).
            // Fall back to model.name when address_segments is empty (e.g. Python models
            // pending Phase 5 address computation).
            let canonical_key = {
                let cp = model.canonical_path();
                if cp.is_empty() {
                    model.name.clone()
                } else {
                    cp
                }
            };

            if let Some(existing) = nodes.get(&canonical_key) {
                let existing: &LogicalNode = existing;
                tracing::warn!(
                    "Duplicate model '{}'. Model at {} overwrites model at {}.",
                    canonical_key,
                    model.path.display(),
                    existing.model_file.path.display()
                );
            }

            nodes.insert(
                canonical_key.clone(),
                LogicalNode {
                    // LogicalNode.name is the canonical path (same as the map key).
                    name: canonical_key.clone(),
                    // Placeholder — filled in during pass 2.
                    dependencies: Vec::new(),
                    materialization,
                    timeseries,
                    incremental,
                    target,
                    tags,
                    model_file: model,
                    // Hand-authored models have no generator provenance.
                    // Generator-emitted models are populated separately when
                    // the generator pipeline feeds into the logical graph.
                    generator_file: None,
                    generator_name: None,
                },
            );

            pending.push(PendingNode {
                canonical_key,
                raw_dep_paths,
            });
        }

        // Pass 2: resolve raw dep paths to canonical node-name keys.
        //
        // Each dep_path is a Vec<String> of path segments from the ref call.
        // Joining them with "." gives the canonical key directly — no address
        // index needed. For a single-segment ref like `smelt.stg_orders` the
        // key is `"stg_orders"` (flat model). For a qualified ref like
        // `smelt.silver.events_parsed` the key is `"silver.events_parsed"`.
        //
        // Unresolved deps are kept as their full dotted path so `validate()` can
        // surface a useful "references undefined model" error.
        for p in pending {
            let deps: Vec<String> = p
                .raw_dep_paths
                .into_iter()
                .map(|segs| {
                    let full = segs.join(".");
                    // Canonical key exact match.
                    if nodes.contains_key(&full) {
                        return full;
                    }
                    // Unresolvable: return full path for a meaningful validate() error.
                    full
                })
                .collect();

            if let Some(node) = nodes.get_mut(&p.canonical_key) {
                node.dependencies = deps;
            }
        }

        Ok(Self {
            nodes,
            sources: source_set,
            seeds: seed_set,
        })
    }

    /// Annotate nodes whose names appear in `origins` with generator provenance.
    ///
    /// `origins` maps the model's smelt-path name (e.g. `"cohorts.us_west"`) to
    /// `(generator_file_rel_path, generator_def_name)`.  Any node name not in
    /// the map is left unchanged (its `generator_file`/`generator_name` remain
    /// `None`).  This is called after `build()` when the emitted-models Salsa
    /// pipeline has determined which survivors exist.
    pub fn annotate_emitted_models(
        &mut self,
        origins: &std::collections::HashMap<String, (String, String)>,
    ) {
        for (name, (gen_file, gen_name)) in origins {
            if let Some(node) = self.nodes.get_mut(name) {
                node.generator_file = Some(gen_file.clone());
                node.generator_name = Some(gen_name.clone());
            }
        }
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
                        vec![]
                    }
                }
                SelectionMethod::Tag(tag) => self
                    .nodes
                    .values()
                    .filter(|node| node.tags.contains(tag))
                    .map(|node| node.name.clone())
                    .collect(),
                // `GeneratorFile` selection requires the emitted-models pipeline.
                // At the LogicalGraph level we match nodes whose `origin` field
                // records the given generator path.
                SelectionMethod::GeneratorFile { path } => {
                    let path_str = path.to_string_lossy();
                    self.nodes
                        .values()
                        .filter(|node| {
                            node.generator_file
                                .as_deref()
                                .map(|gf| gf == path_str.as_ref())
                                .unwrap_or(false)
                        })
                        .map(|node| node.name.clone())
                        .collect()
                }
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
                } else if self.seeds.contains(dep) {
                    // Seeds have no further upstreams — insert without recursing.
                    result.insert(dep.clone());
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
            target: None,
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
                timeseries: None,
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
                timeseries: None,
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

    /// A seed nested under a sub-directory of a scan root. Its leaf `name` is
    /// the file stem, but its canonical address is the full dot-path
    /// (`address_segments.join(".")`). E.g. `models/lookup/regions.csv` →
    /// `smelt.lookup.regions` with `name = "regions"`.
    fn make_subdir_seed(segments: &[&str]) -> smelt_core::SeedInfo {
        let segs: Vec<String> = segments.iter().map(|s| s.to_string()).collect();
        smelt_core::SeedInfo {
            name: segs.last().expect("at least one segment").clone(),
            path: format!("models/{}.csv", segs.join("/")).into(),
            columns: Vec::new(),
            address_segments: segs,
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

    /// Regression: a seed nested under a sub-directory of a scan root must
    /// validate as a `smelt.<full.path>` ref target, mirroring the canonical
    /// addressing the Salsa/LSP resolver uses (architecture.md §Resolution,
    /// cli.md Constraint #11). `models/lookup/regions.csv` is addressable as
    /// `smelt.lookup.regions` — keying the seed set by leaf `name` only
    /// (the prior bug) made the CLI run/explain path reject it while the LSP
    /// example gates passed, the asymmetric-discovery bug class.
    #[test]
    fn test_subdirectory_seed_as_ref_target_validates() {
        let models = vec![make_model("region_report", vec!["lookup.regions"])];
        let seeds = vec![make_subdir_seed(&["lookup", "regions"])];
        let config = make_test_config();

        let graph = LogicalGraph::build(models, None, &seeds, &config, "dev").expect("build graph");

        graph
            .validate()
            .expect("subdirectory seed ref `smelt.lookup.regions` should validate");

        // The seed is surfaced under its canonical dot-path, not its leaf name
        // (cli.md Constraint #10: all CLI output is canonical `smelt.<path>`).
        let seed_names: HashSet<&str> = graph.iter_seeds().collect();
        assert!(
            seed_names.contains("lookup.regions"),
            "subdir seed should be visible via iter_seeds() by canonical path: saw {:?}",
            seed_names
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

    /// Spec Constraint 9: LogicalGraph must key nodes by canonical dot-paths,
    /// not by leaf names. Same-leaf models in different layers must coexist.
    #[test]
    fn logical_graph_keys_by_canonical_path() {
        use smelt_core::ModelId;

        // Build two models with the same leaf name "events" but different layers.
        fn make_layered_model(layer: &str, leaf: &str, deps: Vec<&str>) -> ModelFile {
            let refs = deps
                .into_iter()
                .map(|dep| RefInfo {
                    has_named_params: false,
                    range: TextRange::default(),
                    smelt_ref: smelt_core::refs::SmeltRef::Path(vec![dep.to_string()]),
                })
                .collect();

            let path: std::path::PathBuf = format!("models/{}/{}.sql", layer, leaf).into();
            ModelFile {
                name: leaf.to_string(),
                model_id: ModelId::from_path(path.clone()),
                path,
                content: String::new(),
                refs,
                parse_errors: Vec::new(),
                metadata: None,
                kind: ModelKind::Sql,
                // address_segments drive canonical_path()
                address_segments: vec![layer.to_string(), leaf.to_string()],
            }
        }

        let silver_events = make_layered_model("silver", "events", vec![]);
        let bronze_events = make_layered_model("bronze", "events", vec![]);

        let config = make_test_config();
        let graph = LogicalGraph::build(
            vec![silver_events, bronze_events],
            None,
            &[],
            &config,
            "dev",
        )
        .expect("build graph");

        // Canonical-path keys must be accessible.
        assert!(
            graph.get_model("silver.events").is_ok(),
            "silver.events should be in graph"
        );
        assert!(
            graph.get_model("bronze.events").is_ok(),
            "bronze.events should be in graph"
        );

        // Leaf-only lookup must fail (no longer resolves).
        assert!(
            graph.get_model("events").is_err(),
            "bare leaf 'events' should not resolve after canonical-path rekey"
        );

        // all_model_names() must return canonical paths.
        let names = graph.all_model_names();
        assert!(
            names.contains("silver.events"),
            "all_model_names should contain 'silver.events', got {:?}",
            names
        );
        assert!(
            names.contains("bronze.events"),
            "all_model_names should contain 'bronze.events', got {:?}",
            names
        );
        assert_eq!(names.len(), 2, "should be exactly two models");
    }

    /// BUG-045 regression: upstream traversal must include seeds that the
    /// model depends on via smelt.<path>. Previously `collect_upstream` only
    /// walked `self.nodes`, so seeds (which live in `self.seeds`) were silently
    /// skipped even when present in `node.dependencies`.
    #[test]
    fn select_upstream_includes_seed_dependencies() {
        // Model A depends on seed "raw_data".
        let models = vec![make_model("A", vec!["raw_data"])];
        let seeds = vec![make_seed("raw_data")];
        let config = make_test_config();
        let graph = LogicalGraph::build(models, None, &seeds, &config, "dev").unwrap();

        let selectors = vec![smelt_core::parse_selector("+A").unwrap()];
        let selected = graph.select_models(&selectors).unwrap();

        assert!(selected.contains("A"), "model A should be in the selection");
        assert!(
            selected.contains("raw_data"),
            "seed 'raw_data' should appear in upstream traversal; got: {:?}",
            selected
        );
    }

    /// Spec Semantics — downstream traversal: `model_name+` includes the model
    /// and all its downstream dependents.
    #[test]
    fn select_downstream_includes_dependents() {
        let models = vec![
            make_model("A", vec![]),
            make_model("B", vec!["A"]),
            make_model("C", vec!["B"]),
        ];
        let config = make_test_config();
        let graph = LogicalGraph::build(models, None, &[], &config, "dev").unwrap();

        let selectors = vec![smelt_core::parse_selector("A+").unwrap()];
        let selected = graph.select_models(&selectors).unwrap();

        assert!(selected.contains("A"), "A should be in the selection");
        assert!(
            selected.contains("B"),
            "B (downstream of A) should be included"
        );
        assert!(
            selected.contains("C"),
            "C (downstream of A via B) should be included"
        );
    }

    /// Spec Semantics — union of selectors: multiple `--select` flags produce a
    /// union, not an intersection.
    #[test]
    fn select_multiple_selectors_union() {
        let models = vec![
            make_model("A", vec![]),
            make_model("B", vec![]),
            make_model("C", vec!["A", "B"]),
        ];
        let config = make_test_config();
        let graph = LogicalGraph::build(models, None, &[], &config, "dev").unwrap();

        let selectors = vec![
            smelt_core::parse_selector("A").unwrap(),
            smelt_core::parse_selector("B").unwrap(),
        ];
        let selected = graph.select_models(&selectors).unwrap();

        assert!(selected.contains("A"), "A should be selected");
        assert!(selected.contains("B"), "B should be selected");
        assert!(
            !selected.contains("C"),
            "C not in either selector — should not be selected"
        );
    }

    /// Spec Semantics — exclusion is post-selection: `exclude_models` removes
    /// the specified models from the selected set.
    #[test]
    fn exclude_models_removes_from_selection() {
        let models = vec![
            make_model("A", vec![]),
            make_model("B", vec!["A"]),
            make_model("C", vec!["B"]),
        ];
        let config = make_test_config();
        let graph = LogicalGraph::build(models, None, &[], &config, "dev").unwrap();

        let selectors = vec![smelt_core::parse_selector("+C").unwrap()];
        let selected = graph.select_models(&selectors).unwrap();
        // selected = {A, B, C}

        let excludes = vec![smelt_core::parse_selector("B").unwrap()];
        let filtered = graph.exclude_models(&selected, &excludes).unwrap();

        assert!(filtered.contains("A"), "A should remain after excluding B");
        assert!(!filtered.contains("B"), "B should be excluded");
        assert!(filtered.contains("C"), "C should remain after excluding B");
    }

    /// Spec Semantics — no-match is not an error: a tag selector matching no
    /// models returns an empty set without error.
    #[test]
    fn select_no_match_tag_returns_empty_ok() {
        let models = vec![make_model_with_tags("A", vec![], vec!["core"])];
        let config = make_test_config();
        let graph = LogicalGraph::build(models, None, &[], &config, "dev").unwrap();

        let selectors = vec![smelt_core::parse_selector("tag:nonexistent").unwrap()];
        let selected = graph.select_models(&selectors).unwrap();

        assert!(
            selected.is_empty(),
            "no models have tag:nonexistent; set should be empty"
        );
    }

    /// Spec Semantics — GeneratorFile selector: matches nodes whose
    /// `generator_file` field records the given workspace-relative path.
    #[test]
    fn select_generator_file_matches_emitted_models() {
        let models = vec![
            make_model("emitted_a", vec![]),
            make_model("emitted_b", vec![]),
            make_model("hand_authored", vec![]),
        ];
        let config = make_test_config();
        let mut graph = LogicalGraph::build(models, None, &[], &config, "dev").unwrap();

        // Annotate two nodes as emitted from the same generator file.
        let mut origins = std::collections::HashMap::new();
        origins.insert(
            "emitted_a".to_string(),
            ("models/cohorts.gen.sql".to_string(), "cohort_a".to_string()),
        );
        origins.insert(
            "emitted_b".to_string(),
            ("models/cohorts.gen.sql".to_string(), "cohort_b".to_string()),
        );
        graph.annotate_emitted_models(&origins);

        let selectors =
            vec![smelt_core::parse_selector("generator_file:models/cohorts.gen.sql").unwrap()];
        let selected = graph.select_models(&selectors).unwrap();

        assert!(
            selected.contains("emitted_a"),
            "emitted_a should be selected"
        );
        assert!(
            selected.contains("emitted_b"),
            "emitted_b should be selected"
        );
        assert!(
            !selected.contains("hand_authored"),
            "hand_authored should not be selected"
        );
    }
}
