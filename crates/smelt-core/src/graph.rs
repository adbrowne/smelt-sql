use crate::config::Config;
use crate::discovery::ModelFile;
use crate::selector::{SelectionMethod, Selector};
use crate::SourcesConfig;
use anyhow::{anyhow, Result};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use thiserror::Error;

/// Compute the workspace-relative path tuple for a model file.
///
/// `examples/test_workspace/models/users.sql` with workspace root
/// `examples/test_workspace/` becomes `["models", "users"]`. The leaf
/// segment is the model's `name` (which already accounts for
/// multi-model files where the leaf is taken from frontmatter rather
/// than the filename); intermediate segments are the parent directory
/// components from the workspace root down to the file's parent.
fn path_tuple_for_model(workspace_root: &Path, model: &ModelFile) -> Vec<String> {
    let source_path = model.model_id.source_path();
    let parent = source_path.parent().unwrap_or(Path::new(""));
    // Try to strip the workspace root from the parent to get a workspace-
    // relative directory. If `parent` is not a descendant of `workspace_root`
    // (e.g. the model came from a tempdir or virtual path) fall back to the
    // parent's components verbatim — the resulting tuple is still unique
    // because the leaf model name is appended.
    let rel_dir = parent.strip_prefix(workspace_root).unwrap_or(parent);
    let mut tuple: Vec<String> = rel_dir
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    tuple.push(model.name.clone());
    tuple
}

#[derive(Debug, Error)]
pub enum GraphError {
    #[error("Dependency resolution failed:\n  {message}")]
    DependencyError { message: String },

    #[error("Circular dependency detected involving models: {models}")]
    CircularDependency { models: String },
}

pub struct DependencyGraph {
    /// model_name -> dependencies (model names it references)
    dependencies: HashMap<String, Vec<String>>,
    /// model_name -> ModelFile
    models: HashMap<String, ModelFile>,
    /// External sources (from sources.yml)
    sources: HashSet<String>,
    /// Path-tuple keyed dependency edges (Phase 2a). Empty when the
    /// graph was built via the legacy `build` constructor; populated
    /// by [`DependencyGraph::build_from_workspace`].
    path_dependencies: HashMap<Vec<String>, Vec<Vec<String>>>,
}

impl DependencyGraph {
    pub fn build(models: Vec<ModelFile>, sources: Option<&SourcesConfig>) -> Result<Self> {
        let mut dependencies = HashMap::new();
        let mut models_map: HashMap<String, ModelFile> = HashMap::new();

        // Build source set (schema.table format)
        let mut source_set = HashSet::new();
        if let Some(sources) = sources {
            for source in &sources.sources {
                for table in &source.tables {
                    source_set.insert(format!("{}.{}", source.name, table.name));
                }
            }
        }

        // Build dependency map from path-form refs. Any SmeltRef::Path whose
        // first segment is not "functions" or "sources" is treated as a model
        // dependency; the leaf segment is used as the model name. Path-tuple
        // resolution is also available via `build_from_workspace`.
        for model in models {
            let deps: Vec<String> = model
                .refs
                .iter()
                .filter_map(|r| {
                    let crate::refs::SmeltRef::Path(segs) = &r.smelt_ref;
                    if segs.is_empty() {
                        return None;
                    }
                    // Exclude function call refs (smelt.functions.*) and source
                    // refs (smelt.sources.*); everything else is a model dep.
                    let first = segs[0].as_str();
                    if first == "functions" || first == "sources" {
                        return None;
                    }
                    segs.last().cloned()
                })
                .collect();

            if let Some(existing) = models_map.get(&model.name) {
                eprintln!(
                    "Warning: Duplicate model name '{}'. Model at {} overwrites model at {}.",
                    model.name,
                    model.path.display(),
                    existing.path.display()
                );
            }
            dependencies.insert(model.name.clone(), deps);
            models_map.insert(model.name.clone(), model);
        }

        Ok(Self {
            dependencies,
            models: models_map,
            sources: source_set,
            path_dependencies: HashMap::new(),
        })
    }

    /// Build a path-tuple keyed dependency graph from a workspace.
    ///
    /// Every `smelt.<path>` reference is keyed by the workspace-relative
    /// path tuple of its referent. The path tuple is derived from each
    /// model's location relative to the workspace root:
    /// `models/users.sql` becomes `["models", "users"]`.
    pub fn build_from_workspace(
        models: Vec<ModelFile>,
        sources: Option<&SourcesConfig>,
        workspace_root: &Path,
    ) -> Result<Self> {
        // First, compute path-tuple edges per model.
        let mut path_dependencies: HashMap<Vec<String>, Vec<Vec<String>>> = HashMap::new();
        for model in &models {
            let model_tuple = path_tuple_for_model(workspace_root, model);
            let edges: Vec<Vec<String>> =
                model.refs.iter().map(|r| r.smelt_ref.to_path()).collect();
            path_dependencies.insert(model_tuple, edges);
        }

        // Then construct the legacy string-keyed graph for compatibility.
        let mut graph = Self::build(models, sources)?;
        graph.path_dependencies = path_dependencies;
        Ok(graph)
    }

    /// Returns the path-tuple keyed dependencies for a model, if the
    /// graph was built via [`build_from_workspace`].
    pub fn path_dependencies(&self, key: &[String]) -> Option<&[Vec<String>]> {
        self.path_dependencies.get(key).map(|v| v.as_slice())
    }

    /// Iterate over the path-tuple keyed dependency map. Empty if the
    /// graph was built via [`build`] rather than [`build_from_workspace`].
    pub fn iter_path_dependencies(&self) -> impl Iterator<Item = (&[String], &[Vec<String>])> {
        self.path_dependencies
            .iter()
            .map(|(k, v)| (k.as_slice(), v.as_slice()))
    }

    /// Validate all references exist (either as models or sources)
    pub fn validate(&self) -> Result<()> {
        let mut errors = Vec::new();

        for (model_name, deps) in &self.dependencies {
            for dep in deps {
                // Check if dependency exists as a model or source
                if !self.models.contains_key(dep) && !self.is_source(dep) {
                    errors.push(format!(
                        "Model '{}' references undefined model/source '{}'",
                        model_name, dep
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

    /// Find cross-backend reference edges (no longer errors; cross-engine refs are supported via Parquet).
    pub fn find_cross_backend_edges(
        &self,
        target_assignments: &HashMap<String, String>,
    ) -> Vec<(String, String, String, String)> {
        let mut edges = Vec::new();

        for (model_name, deps) in &self.dependencies {
            let Some(model_target) = target_assignments.get(model_name) else {
                continue;
            };
            for dep in deps {
                if let Some(dep_target) = target_assignments.get(dep) {
                    if model_target != dep_target {
                        edges.push((
                            model_name.clone(),
                            dep.clone(),
                            model_target.clone(),
                            dep_target.clone(),
                        ));
                    }
                }
            }
        }

        edges
    }

    fn is_source(&self, name: &str) -> bool {
        // Check both plain name and schema.table format
        self.sources.contains(name)
            || self
                .sources
                .iter()
                .any(|s| s.ends_with(&format!(".{}", name)))
    }

    /// Topological sort to determine execution order using Kahn's algorithm
    pub fn execution_order(&self) -> Result<Vec<String>> {
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut dependents: HashMap<String, Vec<String>> = HashMap::new();

        // Initialize in-degree for all models
        for model_name in self.models.keys() {
            in_degree.insert(model_name.clone(), 0);
            dependents.insert(model_name.clone(), Vec::new());
        }

        // Count incoming edges (dependencies)
        for (model_name, deps) in &self.dependencies {
            for dep in deps {
                // Only count model dependencies (skip sources)
                if self.models.contains_key(dep) {
                    *in_degree
                        .get_mut(model_name)
                        .expect("all model names were inserted into in_degree") += 1;
                    dependents
                        .get_mut(dep)
                        .expect("all model names were inserted into dependents")
                        .push(model_name.clone());
                }
            }
        }

        // Kahn's algorithm for topological sort
        let mut queue: VecDeque<String> = in_degree
            .iter()
            .filter(|(_, &degree)| degree == 0)
            .map(|(name, _)| name.clone())
            .collect();

        let mut order = Vec::new();

        while let Some(model_name) = queue.pop_front() {
            order.push(model_name.clone());

            // Reduce in-degree for dependents
            if let Some(deps) = dependents.get(&model_name) {
                for dependent in deps {
                    let degree = in_degree
                        .get_mut(dependent)
                        .expect("dependents only contains valid model names");
                    *degree -= 1;

                    if *degree == 0 {
                        queue.push_back(dependent.clone());
                    }
                }
            }
        }

        // Check for cycles
        if order.len() != self.models.len() {
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

    /// Select models matching the given selectors, with optional upstream/downstream expansion.
    ///
    /// Returns the set of selected model names. The result is the union of all selectors.
    pub fn select_models(
        &self,
        selectors: &[Selector],
        config: &Config,
    ) -> Result<HashSet<String>> {
        let mut selected = HashSet::new();
        let dependents = self.build_dependents_map();

        for selector in selectors {
            // Find directly matching models
            let direct_matches: Vec<String> = match &selector.method {
                SelectionMethod::ModelName(name) => {
                    if self.models.contains_key(name) {
                        vec![name.clone()]
                    } else {
                        return Err(anyhow!("Model '{}' not found", name));
                    }
                }
                SelectionMethod::Tag(tag) => self
                    .models
                    .iter()
                    .filter(|(name, model)| {
                        let tags =
                            config.get_tags(name, model.metadata.as_ref().map(|b| b.as_ref()));
                        tags.contains(tag)
                    })
                    .map(|(name, _)| name.clone())
                    .collect(),
                // `GeneratorFile` selection requires the emitted-models pipeline
                // (smelt-db layer). At the DependencyGraph level we return an
                // empty match set — callers that need generator-file selection
                // must use the smelt-db `resolve_generator_file_selector` helper.
                SelectionMethod::GeneratorFile { .. } => vec![],
            };

            for model_name in &direct_matches {
                selected.insert(model_name.clone());
            }

            // Expand upstream
            if selector.include_upstream {
                for model_name in &direct_matches {
                    self.collect_upstream(model_name, &mut selected);
                }
            }

            // Expand downstream
            if selector.include_downstream {
                for model_name in &direct_matches {
                    self.collect_downstream(model_name, &dependents, &mut selected);
                }
            }
        }

        Ok(selected)
    }

    /// Remove models matching the given exclude selectors from the selected set.
    ///
    /// Uses the same matching logic as `select_models` but subtracts instead of adding.
    pub fn exclude_models(
        &self,
        selected: &HashSet<String>,
        excludes: &[Selector],
        config: &Config,
    ) -> Result<HashSet<String>> {
        let to_exclude = self.select_models(excludes, config)?;
        Ok(selected.difference(&to_exclude).cloned().collect())
    }

    /// Collect all upstream dependencies recursively.
    fn collect_upstream(&self, model_name: &str, result: &mut HashSet<String>) {
        if let Some(deps) = self.dependencies.get(model_name) {
            for dep in deps {
                if self.models.contains_key(dep) && result.insert(dep.clone()) {
                    self.collect_upstream(dep, result);
                }
            }
        }
    }

    /// Build a reverse map: model_name -> models that depend on it.
    fn build_dependents_map(&self) -> HashMap<String, Vec<String>> {
        let mut dependents: HashMap<String, Vec<String>> = HashMap::new();
        for (model_name, deps) in &self.dependencies {
            for dep in deps {
                if self.models.contains_key(dep) {
                    dependents
                        .entry(dep.clone())
                        .or_default()
                        .push(model_name.clone());
                }
            }
        }
        dependents
    }

    /// Collect all downstream dependents recursively.
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

    /// Filter execution order to only include selected models.
    pub fn filtered_execution_order(&self, selected: &HashSet<String>) -> Result<Vec<String>> {
        let full_order = self.execution_order()?;
        Ok(full_order
            .into_iter()
            .filter(|name| selected.contains(name))
            .collect())
    }

    pub fn get_model(&self, name: &str) -> Result<&ModelFile> {
        self.models
            .get(name)
            .ok_or_else(|| anyhow!("Model not found: {}", name))
    }

    pub fn model_count(&self) -> usize {
        self.models.len()
    }

    /// Return the set of all model names.
    pub fn all_model_names(&self) -> HashSet<String> {
        self.models.keys().cloned().collect()
    }

    pub fn iter_models(&self) -> impl Iterator<Item = (&str, &ModelFile)> {
        self.models.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn iter_dependencies(&self) -> impl Iterator<Item = (&str, &[String])> {
        self.dependencies
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_slice()))
    }

    pub fn iter_sources(&self) -> impl Iterator<Item = &str> {
        self.sources.iter().map(|s| s.as_str())
    }

    pub fn models(&self) -> &HashMap<String, ModelFile> {
        &self.models
    }

    /// Get the upstream dependencies for a model (model names it references).
    pub fn get_upstream(&self, model_name: &str) -> Vec<String> {
        self.dependencies
            .get(model_name)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|dep| self.models.contains_key(dep))
            .collect()
    }

    /// Collect all upstream dependencies recursively (public wrapper).
    pub fn all_upstream(&self, model_name: &str) -> HashSet<String> {
        let mut result = HashSet::new();
        self.collect_upstream(model_name, &mut result);
        result
    }

    /// Warn if any ephemeral model has no downstream consumers.
    pub fn warn_unused_ephemerals(&self, config: &Config) {
        use crate::config::Materialization;

        let dependents = self.build_dependents_map();
        for (name, model) in &self.models {
            let mat = config.get_materialization_with_metadata(
                name,
                model.metadata.as_ref().map(|b| b.as_ref()),
            );
            if mat == Materialization::Ephemeral {
                let has_consumers = dependents.get(name).is_some_and(|d| !d.is_empty());
                if !has_consumers {
                    eprintln!(
                        "Warning: Ephemeral model '{}' has no downstream consumers and will never be inlined.",
                        name
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::RefInfo;
    use rowan::TextRange;

    fn make_model(name: &str, deps: Vec<&str>) -> ModelFile {
        let refs = deps
            .into_iter()
            .map(|dep| RefInfo {
                model_name: dep.to_string(),
                has_named_params: false,
                range: TextRange::default(),
                smelt_ref: crate::refs::SmeltRef::Path(vec!["models".to_string(), dep.to_string()]),
            })
            .collect();

        let path: std::path::PathBuf = format!("{}.sql", name).into();
        ModelFile {
            name: name.to_string(),
            model_id: crate::model_id::ModelId::from_path(path.clone()),
            path,
            content: String::new(),
            refs,
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            address_segments: Vec::new(),
        }
    }

    #[test]
    fn test_linear_dependency() {
        // A -> B -> C
        let models = vec![
            make_model("C", vec!["B"]),
            make_model("B", vec!["A"]),
            make_model("A", vec![]),
        ];

        let graph = DependencyGraph::build(models, None).unwrap();
        graph.validate().unwrap();

        let order = graph.execution_order().unwrap();
        assert_eq!(order, vec!["A", "B", "C"]);
    }

    #[test]
    fn test_diamond_dependency() {
        //     A
        //    / \
        //   B   C
        //    \ /
        //     D
        let models = vec![
            make_model("D", vec!["B", "C"]),
            make_model("C", vec!["A"]),
            make_model("B", vec!["A"]),
            make_model("A", vec![]),
        ];

        let graph = DependencyGraph::build(models, None).unwrap();
        graph.validate().unwrap();

        let order = graph.execution_order().unwrap();
        assert_eq!(order.len(), 4);
        assert_eq!(order[0], "A");
        assert_eq!(order[3], "D");
        // B and C can be in either order
        assert!(order[1] == "B" || order[1] == "C");
        assert!(order[2] == "B" || order[2] == "C");
        assert_ne!(order[1], order[2]);
    }

    #[test]
    fn test_circular_dependency() {
        // A -> B -> C -> A
        let models = vec![
            make_model("A", vec!["C"]),
            make_model("B", vec!["A"]),
            make_model("C", vec!["B"]),
        ];

        let graph = DependencyGraph::build(models, None).unwrap();
        let result = graph.execution_order();

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Circular dependency"));
    }

    #[test]
    fn test_undefined_reference() {
        let models = vec![make_model("A", vec!["nonexistent"])];

        let graph = DependencyGraph::build(models, None).unwrap();
        let result = graph.validate();

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("undefined"));
        assert!(err_msg.contains("nonexistent"));
    }

    /// Layer-prefix refs like `smelt.silver.events_parsed` produce segments
    /// `["silver", "events_parsed"]`. The graph must capture these as model
    /// dependencies so that UI edges and execution order are correct.
    #[test]
    fn test_layer_prefix_refs_captured_as_deps() {
        // Simulate smelt.silver.events_parsed / smelt.bronze.raw_events style refs.
        let make_layer_ref = |layer: &str, model: &str| RefInfo {
            model_name: model.to_string(),
            has_named_params: false,
            range: TextRange::default(),
            smelt_ref: crate::refs::SmeltRef::Path(vec![layer.to_string(), model.to_string()]),
        };
        let make_function_ref = |name: &str| RefInfo {
            model_name: name.to_string(),
            has_named_params: false,
            range: TextRange::default(),
            smelt_ref: crate::refs::SmeltRef::Path(vec![
                "functions".to_string(),
                name.to_string(),
            ]),
        };

        let mut raw_events = make_model("raw_events", vec![]);
        let mut events_parsed = make_model("events_parsed", vec![]);
        events_parsed.refs = vec![
            make_layer_ref("bronze", "raw_events"),
            make_function_ref("parse_event_payload"), // should be excluded
        ];
        let mut sessions = make_model("sessions", vec![]);
        sessions.refs = vec![make_layer_ref("silver", "events_parsed")];
        raw_events.refs = vec![];

        let graph = DependencyGraph::build(vec![raw_events, events_parsed, sessions], None).unwrap();

        let order = graph.execution_order().unwrap();
        assert_eq!(order[0], "raw_events");
        assert_eq!(order[1], "events_parsed");
        assert_eq!(order[2], "sessions");

        let deps = graph.get_upstream("events_parsed");
        assert_eq!(deps, vec!["raw_events"]);

        let deps2 = graph.get_upstream("sessions");
        assert_eq!(deps2, vec!["events_parsed"]);
    }

    #[test]
    fn test_source_reference() {
        use crate::{SourceColumnDef, SourceDef, SourceTableDef, SourcesConfig};

        let models = vec![make_model("A", vec!["source.events"])];

        let source_config = SourcesConfig {
            sources: vec![SourceDef {
                name: "source".to_string(),
                database: None,
                schema: None,
                description: None,
                tables: vec![SourceTableDef {
                    name: "events".to_string(),
                    identifier: None,
                    description: None,
                    columns: vec![SourceColumnDef {
                        name: "id".to_string(),
                        data_type: None,
                        description: None,
                        data_latency: None,
                    }],
                }],
            }],
        };

        let graph = DependencyGraph::build(models, Some(&source_config)).unwrap();
        assert!(graph.validate().is_ok());
    }

    fn make_model_with_tags(name: &str, deps: Vec<&str>, tags: Vec<&str>) -> ModelFile {
        let refs = deps
            .into_iter()
            .map(|dep| RefInfo {
                model_name: dep.to_string(),
                has_named_params: false,
                range: TextRange::default(),
                smelt_ref: crate::refs::SmeltRef::Path(vec!["models".to_string(), dep.to_string()]),
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
            model_id: crate::model_id::ModelId::from_path(path.clone()),
            path,
            content: String::new(),
            refs,
            parse_errors: Vec::new(),
            metadata,
            kind: crate::discovery::ModelKind::Sql,
            address_segments: Vec::new(),
        }
    }

    fn make_test_config_with_tags(model_tags: Vec<(&str, Vec<&str>)>) -> Config {
        use crate::config::{Materialization, ModelConfig, Target};

        let mut models = HashMap::new();
        for (name, tags) in model_tags {
            models.insert(
                name.to_string(),
                ModelConfig {
                    materialization: None,
                    timeseries: None,
                    incremental: None,
                    tags: tags.into_iter().map(|t| t.to_string()).collect(),
                    target: None,
                },
            );
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
        }
    }

    #[test]
    fn test_select_by_model_name() {
        let models = vec![
            make_model("A", vec![]),
            make_model("B", vec!["A"]),
            make_model("C", vec!["B"]),
        ];
        let graph = DependencyGraph::build(models, None).unwrap();
        let config = make_test_config_with_tags(vec![]);

        let selectors = vec![crate::selector::parse_selector("B").unwrap()];
        let selected = graph.select_models(&selectors, &config).unwrap();

        assert_eq!(selected.len(), 1);
        assert!(selected.contains("B"));
    }

    #[test]
    fn test_select_by_tag() {
        let models = vec![
            make_model_with_tags("A", vec![], vec!["core"]),
            make_model_with_tags("B", vec!["A"], vec!["revenue"]),
            make_model_with_tags("C", vec!["B"], vec!["revenue", "daily"]),
        ];
        let graph = DependencyGraph::build(models, None).unwrap();
        let config = make_test_config_with_tags(vec![]);

        let selectors = vec![crate::selector::parse_selector("tag:revenue").unwrap()];
        let selected = graph.select_models(&selectors, &config).unwrap();

        assert_eq!(selected.len(), 2);
        assert!(selected.contains("B"));
        assert!(selected.contains("C"));
    }

    #[test]
    fn test_select_tag_from_config() {
        // Tags defined in smelt.yml config, not frontmatter
        let models = vec![make_model("A", vec![]), make_model("B", vec!["A"])];
        let graph = DependencyGraph::build(models, None).unwrap();
        let config = make_test_config_with_tags(vec![("B", vec!["important"])]);

        let selectors = vec![crate::selector::parse_selector("tag:important").unwrap()];
        let selected = graph.select_models(&selectors, &config).unwrap();

        assert_eq!(selected.len(), 1);
        assert!(selected.contains("B"));
    }

    #[test]
    fn test_select_upstream() {
        // A -> B -> C
        let models = vec![
            make_model("A", vec![]),
            make_model("B", vec!["A"]),
            make_model("C", vec!["B"]),
        ];
        let graph = DependencyGraph::build(models, None).unwrap();
        let config = make_test_config_with_tags(vec![]);

        let selectors = vec![crate::selector::parse_selector("+C").unwrap()];
        let selected = graph.select_models(&selectors, &config).unwrap();

        assert_eq!(selected.len(), 3);
        assert!(selected.contains("A"));
        assert!(selected.contains("B"));
        assert!(selected.contains("C"));
    }

    #[test]
    fn test_select_downstream() {
        // A -> B -> C
        let models = vec![
            make_model("A", vec![]),
            make_model("B", vec!["A"]),
            make_model("C", vec!["B"]),
        ];
        let graph = DependencyGraph::build(models, None).unwrap();
        let config = make_test_config_with_tags(vec![]);

        let selectors = vec![crate::selector::parse_selector("A+").unwrap()];
        let selected = graph.select_models(&selectors, &config).unwrap();

        assert_eq!(selected.len(), 3);
        assert!(selected.contains("A"));
        assert!(selected.contains("B"));
        assert!(selected.contains("C"));
    }

    #[test]
    fn test_select_tag_with_upstream() {
        // A -> B(tagged) -> C
        let models = vec![
            make_model("A", vec![]),
            make_model_with_tags("B", vec!["A"], vec!["target"]),
            make_model("C", vec!["B"]),
        ];
        let graph = DependencyGraph::build(models, None).unwrap();
        let config = make_test_config_with_tags(vec![]);

        let selectors = vec![crate::selector::parse_selector("+tag:target").unwrap()];
        let selected = graph.select_models(&selectors, &config).unwrap();

        assert_eq!(selected.len(), 2);
        assert!(selected.contains("A"));
        assert!(selected.contains("B"));
    }

    #[test]
    fn test_filtered_execution_order() {
        let models = vec![
            make_model("A", vec![]),
            make_model("B", vec!["A"]),
            make_model("C", vec!["B"]),
        ];
        let graph = DependencyGraph::build(models, None).unwrap();

        let mut selected = HashSet::new();
        selected.insert("A".to_string());
        selected.insert("C".to_string());

        let order = graph.filtered_execution_order(&selected).unwrap();
        assert_eq!(order, vec!["A", "C"]);
    }

    #[test]
    fn test_tag_merge_dedup() {
        // Tag "shared" in both frontmatter and config
        let models = vec![make_model_with_tags(
            "A",
            vec![],
            vec!["shared", "from_sql"],
        )];
        let graph = DependencyGraph::build(models, None).unwrap();
        let config = make_test_config_with_tags(vec![("A", vec!["shared", "from_yml"])]);

        let model = graph.get_model("A").unwrap();
        let tags = config.get_tags("A", model.metadata.as_ref().map(|b| b.as_ref()));

        // "shared" from config, "from_yml" from config, "from_sql" from frontmatter
        // "shared" should NOT be duplicated
        assert_eq!(tags.len(), 3);
        assert!(tags.contains(&"shared".to_string()));
        assert!(tags.contains(&"from_yml".to_string()));
        assert!(tags.contains(&"from_sql".to_string()));
    }

    #[test]
    fn test_select_nonexistent_model() {
        let models = vec![make_model("A", vec![])];
        let graph = DependencyGraph::build(models, None).unwrap();
        let config = make_test_config_with_tags(vec![]);

        let selectors = vec![crate::selector::parse_selector("nonexistent").unwrap()];
        let result = graph.select_models(&selectors, &config);

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("nonexistent"));
        assert!(err_msg.contains("not found"));
    }

    #[test]
    fn test_cross_backend_edges_same_target_empty() {
        let models = vec![
            make_model("upstream", vec![]),
            make_model("downstream", vec!["upstream"]),
        ];
        let graph = DependencyGraph::build(models, None).unwrap();

        let mut assignments = HashMap::new();
        assignments.insert("upstream".to_string(), "dev".to_string());
        assignments.insert("downstream".to_string(), "dev".to_string());

        assert!(graph.find_cross_backend_edges(&assignments).is_empty());
    }

    #[test]
    fn test_cross_backend_edges_different_target_detected() {
        let models = vec![
            make_model("upstream", vec![]),
            make_model("downstream", vec!["upstream"]),
        ];
        let graph = DependencyGraph::build(models, None).unwrap();

        let mut assignments = HashMap::new();
        assignments.insert("upstream".to_string(), "spark_prod".to_string());
        assignments.insert("downstream".to_string(), "dev".to_string());

        let edges = graph.find_cross_backend_edges(&assignments);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].0, "downstream");
        assert_eq!(edges[0].1, "upstream");
    }

    #[test]
    fn test_cross_backend_edges_independent_models_empty() {
        let models = vec![make_model("model_a", vec![]), make_model("model_b", vec![])];
        let graph = DependencyGraph::build(models, None).unwrap();

        let mut assignments = HashMap::new();
        assignments.insert("model_a".to_string(), "dev".to_string());
        assignments.insert("model_b".to_string(), "spark_prod".to_string());

        // Independent models on different targets have no cross-backend edges
        assert!(graph.find_cross_backend_edges(&assignments).is_empty());
    }
}
