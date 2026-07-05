use crate::config::Config;
use crate::discovery::ModelFile;
use crate::selector::{SelectionMethod, Selector};
use crate::SourcesConfig;
use anyhow::{anyhow, Result};
use std::collections::{HashMap, HashSet, VecDeque};
use thiserror::Error;
use tracing::warn;

#[derive(Debug, Error)]
pub enum GraphError {
    #[error("Dependency resolution failed:\n  {message}")]
    DependencyError { message: String },

    #[error("Circular dependency detected involving models: {models}")]
    CircularDependency { models: String },
}

pub struct DependencyGraph {
    /// canonical_path -> dependencies (canonical paths it references)
    ///
    /// Keys and values are canonical dot-path strings, e.g. `"silver.events"`.
    /// This is the structural enforcement of Invariant 9: the graph is keyed
    /// exclusively on canonical paths; leaf-only names are not stored here.
    dependencies: HashMap<String, Vec<String>>,
    /// canonical_path -> ModelFile
    models: HashMap<String, ModelFile>,
    /// External sources (from sources.yml)
    sources: HashSet<String>,
}

impl DependencyGraph {
    /// Build a `DependencyGraph` from a list of `ModelFile`s.
    ///
    /// Both `dependencies` and `models` maps are keyed by canonical dot-path
    /// strings (e.g. `"silver.events"`). This enforces Invariant 9: leaf-only
    /// model names exist only as a parsed-out field of `ModelFile` for
    /// diagnostic context, not as a resolution key.
    ///
    /// Dep edges are also canonical dot-paths: a ref `smelt.silver.events`
    /// produces segments `["silver", "events"]` which join to `"silver.events"`.
    /// Refs whose first segment is `"functions"` or `"sources"` are excluded.
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
        // first segment is not "functions", "sources", or "seeds" is treated
        // as a model dependency; the full dot-joined path is used as the key.
        for model in models {
            let canonical = model.canonical_path();
            let deps: Vec<String> = model
                .refs
                .iter()
                .filter_map(|r| {
                    let crate::refs::SmeltRef::Path(segs) = &r.smelt_ref;
                    if segs.is_empty() {
                        return None;
                    }
                    // Exclude function call refs (smelt.functions.*), source
                    // refs (smelt.sources.*), seed refs (smelt.seeds.*), and
                    // meta-language compile-time calls (smelt.config.* — e.g.
                    // `smelt.config.var`, `smelt.config.load_yaml`), which the
                    // build-path meta evaluator resolves before codegen and
                    // which the analyzer likewise does not treat as model deps;
                    // everything else is a model dep.
                    let first = segs[0].as_str();
                    if first == "functions"
                        || first == "sources"
                        || first == "seeds"
                        || first == "config"
                    {
                        return None;
                    }
                    // `smelt.models.with_tag(...)` / `smelt.models.all` are
                    // wide-reflection accessors (compile-time meta resolved by
                    // the build-path evaluator), not model deps — distinct from
                    // a legacy `smelt.models.<leaf>` model reference.
                    if first == "models"
                        && matches!(
                            segs.get(1).map(String::as_str),
                            Some("with_tag") | Some("all")
                        )
                    {
                        return None;
                    }
                    let dep = segs.join(".");
                    // A batched model may read its own prior partitions
                    // (`smelt.<self>` — `docs/specs/batched_models.md`
                    // §"Window independence and self-referential models").
                    // That self-edge is not a topological dependency (the
                    // model already exists by definition), so it must not
                    // induce a cycle here; whether the self-reference
                    // actually *converges* partition-by-partition is a
                    // separate, later planner check
                    // (`window_independence`/BL7), not this graph's concern.
                    if dep == canonical {
                        return None;
                    }
                    Some(dep)
                })
                .collect();

            if let Some(existing) = models_map.get(&canonical) {
                warn!(
                    "duplicate canonical path '{}': model at {} overwrites model at {}",
                    canonical,
                    model.path.display(),
                    existing.path.display()
                );
            }
            dependencies.insert(canonical.clone(), deps);
            models_map.insert(canonical, model);
        }

        Ok(Self {
            dependencies,
            models: models_map,
            sources: source_set,
        })
    }

    /// Add seed names as valid `smelt.ref()` targets so that `validate()` does
    /// not report them as undefined. Seeds are CSV files (or sidecar-declared)
    /// that are valid ref targets but are not SQL models or external sources.
    ///
    /// Called before `validate()` when the caller has discovered seeds via
    /// `smelt_core::discover_seed_infos[_with_sidecars]`.
    pub fn add_seeds(&mut self, seeds: &[crate::SeedInfo]) {
        for seed in seeds {
            // Seeds are addressable by their canonical dot-path (address_segments.join("."))
            // and by their leaf name. Add both forms so both resolution styles work.
            let cp = seed.address_segments.join(".");
            if !cp.is_empty() {
                self.sources.insert(cp);
            }
            if !seed.name.is_empty() {
                self.sources.insert(seed.name.clone());
            }
        }
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
                // Match emitted models whose virtual path was produced by the
                // named generator file. Virtual paths have the form
                // `<abs-gen-dir>/<gen-filename>::<smelt-name>` — the `::` marker
                // distinguishes them from hand-authored models. We strip the
                // `<smelt-name>` suffix and check if the remainder ends with the
                // workspace-relative selector path (forward- or OS-slash).
                SelectionMethod::GeneratorFile { path } => {
                    let sel_fwd = path.to_string_lossy().replace('\\', "/");
                    let sel_os = path
                        .to_string_lossy()
                        .replace('/', std::path::MAIN_SEPARATOR_STR);
                    self.models
                        .iter()
                        .filter(|(_, model)| {
                            let p = model.path.to_string_lossy();
                            if let Some(gen_path_str) = p.split("::").next() {
                                let gen_fwd = gen_path_str.replace('\\', "/");
                                gen_fwd.ends_with(&*sel_fwd) || gen_fwd.ends_with(&*sel_os)
                            } else {
                                false
                            }
                        })
                        .map(|(name, _)| name.clone())
                        .collect()
                }
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

    /// Check that every model in `selected` has all its direct built-model
    /// dependencies also present in `selected`.
    ///
    /// Returns `(retained_model, missing_dep)` pairs where a retained model's
    /// direct model dependency was removed from the working set (e.g. by
    /// `--exclude +model` upstream expansion). Sources and seeds are never
    /// flagged — only model-to-model dependency edges.
    pub fn check_working_set_consistency(
        &self,
        selected: &HashSet<String>,
    ) -> Vec<(String, String)> {
        let mut violations = Vec::new();
        for model in selected {
            if let Some(deps) = self.dependencies.get(model) {
                for dep in deps {
                    if self.models.contains_key(dep) && !selected.contains(dep) {
                        violations.push((model.clone(), dep.clone()));
                    }
                }
            }
        }
        violations
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
                    warn!(
                        "ephemeral model '{}' has no downstream consumers and will never be inlined",
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

    /// Make a flat model (no layer prefix). canonical_path() == name.
    ///
    /// Each dep in `deps` is expected to be a canonical path string (dot-joined).
    /// The ref path is split on `.` to produce segments matching `segs.join(".")`.
    fn make_model(name: &str, deps: Vec<&str>) -> ModelFile {
        let refs = deps
            .into_iter()
            .map(|dep| {
                // Split the dep canonical path into segments so that
                // `segs.join(".")` == dep. Single-segment deps like "A"
                // produce `["A"]`; layered deps like "silver.events" produce
                // `["silver", "events"]`.
                let segs: Vec<String> = dep.split('.').map(|s| s.to_string()).collect();
                RefInfo {
                    has_named_params: false,
                    range: TextRange::default(),
                    smelt_ref: crate::refs::SmeltRef::Path(segs),
                }
            })
            .collect();

        let path: std::path::PathBuf = format!("models/{}.sql", name).into();
        ModelFile {
            name: name.to_string(),
            model_id: crate::model_id::ModelId::from_path(path.clone()),
            path,
            content: String::new(),
            refs,
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            // Single-segment address: canonical_path() == name.
            address_segments: vec![name.to_string()],
        }
    }

    /// Make a model with a layer prefix. canonical_path() == "<layer>.<name>".
    fn make_layered_model(layer: &str, name: &str) -> ModelFile {
        let path: std::path::PathBuf = format!("models/{}/{}.sql", layer, name).into();
        ModelFile {
            name: name.to_string(),
            model_id: crate::model_id::ModelId::from_path(path.clone()),
            path,
            content: String::new(),
            refs: Vec::new(),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            address_segments: vec![layer.to_string(), name.to_string()],
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

    /// A self-referencing model (`smelt.<self>`, e.g. a running-balance batched
    /// model — `batched_models.md` §"Window independence and self-referential
    /// models") must not be flagged as a circular dependency: the self-edge is
    /// not a topological dependency, and whether it actually *converges*
    /// partition-by-partition is a separate, later planner check (BL7), not
    /// this graph's concern.
    #[test]
    fn test_self_reference_is_not_a_circular_dependency() {
        let models = vec![make_model("A", vec!["A"])];

        let graph = DependencyGraph::build(models, None).unwrap();
        let order = graph
            .execution_order()
            .expect("a self-referencing model must not be treated as a cycle");
        assert_eq!(order, vec!["A".to_string()]);
    }

    #[test]
    fn test_undefined_reference() {
        // make_model wraps deps in ["models", dep], so the canonical dep key
        // will be "models.nonexistent".
        let models = vec![make_model("A", vec!["nonexistent"])];

        let graph = DependencyGraph::build(models, None).unwrap();
        let result = graph.validate();

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("undefined"));
        assert!(err_msg.contains("nonexistent"));
    }

    /// Layer-prefix refs like `smelt.silver.events_parsed` produce segments
    /// `["silver", "events_parsed"]`. The graph must capture these as canonical
    /// dot-path deps so that UI edges and execution order are correct.
    #[test]
    fn test_layer_prefix_refs_captured_as_deps() {
        // Simulate smelt.silver.events_parsed / smelt.bronze.raw_events style
        // refs. Models are keyed by canonical path (layer.name).
        let make_layer_ref = |layer: &str, model: &str| RefInfo {
            has_named_params: false,
            range: TextRange::default(),
            smelt_ref: crate::refs::SmeltRef::Path(vec![layer.to_string(), model.to_string()]),
        };
        let make_function_ref = |name: &str| RefInfo {
            has_named_params: false,
            range: TextRange::default(),
            smelt_ref: crate::refs::SmeltRef::Path(vec!["functions".to_string(), name.to_string()]),
        };

        let mut raw_events = make_layered_model("bronze", "raw_events");
        let mut events_parsed = make_layered_model("silver", "events_parsed");
        events_parsed.refs = vec![
            make_layer_ref("bronze", "raw_events"),
            make_function_ref("parse_event_payload"), // should be excluded
        ];
        let mut sessions = make_layered_model("gold", "sessions");
        sessions.refs = vec![make_layer_ref("silver", "events_parsed")];
        raw_events.refs = vec![];

        let graph =
            DependencyGraph::build(vec![raw_events, events_parsed, sessions], None).unwrap();

        let order = graph.execution_order().unwrap();
        assert_eq!(order[0], "bronze.raw_events");
        assert_eq!(order[1], "silver.events_parsed");
        assert_eq!(order[2], "gold.sessions");

        let deps = graph.get_upstream("silver.events_parsed");
        assert_eq!(deps, vec!["bronze.raw_events"]);

        let deps2 = graph.get_upstream("gold.sessions");
        assert_eq!(deps2, vec!["silver.events_parsed"]);
    }

    #[test]
    fn test_source_reference() {
        use crate::{SourceColumnDef, SourceDef, SourceTableDef, SourcesConfig};

        // A model that refs smelt.sources.src.events — the "sources" prefix
        // excludes it from model dep tracking, so validate() should succeed
        // even though there's no model named "src.events".
        let mut model_a = make_model("A", vec![]);
        model_a.refs = vec![RefInfo {
            has_named_params: false,
            range: TextRange::default(),
            smelt_ref: crate::refs::SmeltRef::Path(vec![
                "sources".to_string(),
                "src".to_string(),
                "events".to_string(),
            ]),
        }];

        let source_config = SourcesConfig {
            sources: vec![SourceDef {
                name: "src".to_string(),
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

        let graph = DependencyGraph::build(vec![model_a], Some(&source_config)).unwrap();
        assert!(graph.validate().is_ok());
    }

    fn make_model_with_tags(name: &str, deps: Vec<&str>, tags: Vec<&str>) -> ModelFile {
        let refs = deps
            .into_iter()
            .map(|dep| {
                let segs: Vec<String> = dep.split('.').map(|s| s.to_string()).collect();
                RefInfo {
                    has_named_params: false,
                    range: TextRange::default(),
                    smelt_ref: crate::refs::SmeltRef::Path(segs),
                }
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

        let path: std::path::PathBuf = format!("models/{}.sql", name).into();
        ModelFile {
            name: name.to_string(),
            model_id: crate::model_id::ModelId::from_path(path.clone()),
            path,
            content: String::new(),
            refs,
            parse_errors: Vec::new(),
            metadata,
            kind: crate::discovery::ModelKind::Sql,
            // Single-segment address: canonical_path() == name.
            address_segments: vec![name.to_string()],
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
                    refresh: None,
                    batched: None,
                    tags: tags.into_iter().map(|t| t.to_string()).collect(),
                    target: None,
                    format: None,
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

    // ----- canonical-path rekey tests (Phase 3) -------------------------

    /// Two models with the same leaf name but different layer prefixes must
    /// coexist as distinct entries in the graph, keyed by canonical path.
    #[test]
    fn same_leaf_distinct_canonical_paths_coexist() {
        let silver_events = make_layered_model("silver", "events");
        let bronze_events = make_layered_model("bronze", "events");
        let graph = DependencyGraph::build(vec![silver_events, bronze_events], None).unwrap();

        // Both canonical paths must be present.
        assert!(
            graph.get_model("silver.events").is_ok(),
            "silver.events not found"
        );
        assert!(
            graph.get_model("bronze.events").is_ok(),
            "bronze.events not found"
        );

        // They must be distinct models.
        let s = graph.get_model("silver.events").unwrap();
        let b = graph.get_model("bronze.events").unwrap();
        assert_ne!(
            s.path, b.path,
            "silver.events and bronze.events should be distinct models"
        );

        assert_eq!(graph.model_count(), 2);
    }

    /// A model at `models/gold/daily.sql` (canonical `"gold.daily"`) with a
    /// ref `smelt.silver.events_parsed` must record the dep as canonical path
    /// `"silver.events_parsed"` in `graph.dependencies()`.
    #[test]
    fn dependencies_use_canonical_paths() {
        let mut daily = make_layered_model("gold", "daily");
        // Add ref to silver.events_parsed
        daily.refs = vec![RefInfo {
            has_named_params: false,
            range: TextRange::default(),
            smelt_ref: crate::refs::SmeltRef::Path(vec![
                "silver".to_string(),
                "events_parsed".to_string(),
            ]),
        }];
        let events_parsed = make_layered_model("silver", "events_parsed");
        let graph = DependencyGraph::build(vec![daily, events_parsed], None).unwrap();

        // The dep for "gold.daily" must be the canonical path "silver.events_parsed".
        let deps: Vec<String> = graph.get_upstream("gold.daily");
        assert_eq!(
            deps,
            vec!["silver.events_parsed".to_string()],
            "dep key must be canonical path, got: {deps:?}"
        );
    }

    // ----- check_working_set_consistency tests (D-39) -------------------

    /// Retained model has a direct dep that was excluded → violation reported.
    #[test]
    fn consistency_check_flags_missing_dep() {
        // A → B → C.  Select {A, B, C}, then exclude B.
        // C depends on B (excluded) → violation.
        let models = vec![
            make_model("A", vec![]),
            make_model("B", vec!["A"]),
            make_model("C", vec!["B"]),
        ];
        let graph = DependencyGraph::build(models, None).unwrap();

        // Selected = {A, C} (B was excluded).
        let selected: HashSet<String> = ["A".to_string(), "C".to_string()].into();
        let violations = graph.check_working_set_consistency(&selected);

        assert_eq!(
            violations.len(),
            1,
            "expected exactly one violation: {:?}",
            violations
        );
        let (retained, missing) = &violations[0];
        assert_eq!(retained, "C");
        assert_eq!(missing, "B");
    }

    /// All retained models have their deps present → no violations.
    #[test]
    fn consistency_check_clean_set_no_violations() {
        let models = vec![
            make_model("A", vec![]),
            make_model("B", vec!["A"]),
            make_model("C", vec!["B"]),
        ];
        let graph = DependencyGraph::build(models, None).unwrap();

        let selected: HashSet<String> = ["A".to_string(), "B".to_string(), "C".to_string()].into();
        assert!(graph.check_working_set_consistency(&selected).is_empty());
    }

    /// Sources/seeds in the dep list are NOT flagged even when absent from
    /// `selected` — the check is model-to-model only.
    #[test]
    fn consistency_check_ignores_source_deps() {
        use crate::{SourceColumnDef, SourceDef, SourceTableDef, SourcesConfig};

        // model A refs smelt.sources.src.events (a source, not a model).
        let mut model_a = make_model("A", vec![]);
        model_a.refs = vec![RefInfo {
            has_named_params: false,
            range: TextRange::default(),
            smelt_ref: crate::refs::SmeltRef::Path(vec![
                "sources".to_string(),
                "src".to_string(),
                "events".to_string(),
            ]),
        }];

        let source_config = SourcesConfig {
            sources: vec![SourceDef {
                name: "src".to_string(),
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

        let graph = DependencyGraph::build(vec![model_a], Some(&source_config)).unwrap();
        let selected: HashSet<String> = ["A".to_string()].into();
        // src.events is a source, not in self.models → no violation.
        assert!(graph.check_working_set_consistency(&selected).is_empty());
    }

    /// `execution_order()` returns canonical dot-paths in DAG order.
    #[test]
    fn execution_order_uses_canonical_paths() {
        // bronze.raw → silver.parsed → gold.summary
        let raw = make_layered_model("bronze", "raw");
        let mut parsed = make_layered_model("silver", "parsed");
        parsed.refs = vec![RefInfo {
            has_named_params: false,
            range: TextRange::default(),
            smelt_ref: crate::refs::SmeltRef::Path(vec!["bronze".to_string(), "raw".to_string()]),
        }];
        let mut summary = make_layered_model("gold", "summary");
        summary.refs = vec![RefInfo {
            has_named_params: false,
            range: TextRange::default(),
            smelt_ref: crate::refs::SmeltRef::Path(vec![
                "silver".to_string(),
                "parsed".to_string(),
            ]),
        }];

        let graph = DependencyGraph::build(vec![raw, parsed, summary], None).unwrap();
        let order = graph.execution_order().unwrap();

        assert_eq!(order, vec!["bronze.raw", "silver.parsed", "gold.summary"],);
    }
}
