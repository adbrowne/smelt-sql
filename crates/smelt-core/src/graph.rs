use crate::config::Config;
use crate::discovery::ModelFile;
use crate::selector::{SelectionMethod, Selector};
use crate::SourcesConfig;
use anyhow::{anyhow, Result};
use std::collections::{HashMap, HashSet, VecDeque};
use thiserror::Error;

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
}

impl DependencyGraph {
    pub fn build(models: Vec<ModelFile>, sources: Option<&SourcesConfig>) -> Result<Self> {
        let mut dependencies = HashMap::new();
        let mut models_map = HashMap::new();

        // Build source set (schema.table format)
        let mut source_set = HashSet::new();
        if let Some(sources) = sources {
            for source in &sources.sources {
                for table in &source.tables {
                    source_set.insert(format!("{}.{}", source.name, table.name));
                }
            }
        }

        // Build dependency map
        for model in models {
            let deps: Vec<String> = model.refs.iter().map(|r| r.model_name.clone()).collect();

            dependencies.insert(model.name.clone(), deps);
            models_map.insert(model.name.clone(), model);
        }

        Ok(Self {
            dependencies,
            models: models_map,
            sources: source_set,
        })
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
                    *in_degree.get_mut(model_name).unwrap() += 1;
                    dependents.get_mut(dep).unwrap().push(model_name.clone());
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
                    let degree = in_degree.get_mut(dependent).unwrap();
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
            })
            .collect();

        ModelFile {
            name: name.to_string(),
            path: format!("{}.sql", name).into(),
            content: String::new(),
            refs,
            parse_errors: Vec::new(),
            metadata: None,
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

        ModelFile {
            name: name.to_string(),
            path: format!("{}.sql", name).into(),
            content: String::new(),
            refs,
            parse_errors: Vec::new(),
            metadata,
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
                    incremental: None,
                    tags: tags.into_iter().map(|t| t.to_string()).collect(),
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
            },
        );

        Config {
            name: "test".to_string(),
            version: 1,
            model_paths: vec!["models".to_string()],
            seed_paths: vec!["seeds".to_string()],
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
}
