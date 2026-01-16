use crate::config::SourceConfig;
use crate::discovery::ModelFile;
use crate::errors::CliError;
use anyhow::{anyhow, Result};
use std::collections::{HashMap, HashSet, VecDeque};

pub struct DependencyGraph {
    /// model_name -> dependencies (model names it references)
    dependencies: HashMap<String, Vec<String>>,
    /// model_name -> ModelFile
    models: HashMap<String, ModelFile>,
    /// External sources (from sources.yml)
    sources: HashSet<String>,
}

impl DependencyGraph {
    pub fn build(models: Vec<ModelFile>, sources: Option<&SourceConfig>) -> Result<Self> {
        let mut dependencies = HashMap::new();
        let mut models_map = HashMap::new();

        // Build source set (schema.table format)
        let mut source_set = HashSet::new();
        if let Some(sources) = sources {
            for (schema_name, schema) in &sources.sources {
                for table_name in schema.tables.keys() {
                    source_set.insert(format!("{}.{}", schema_name, table_name));
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
            return Err(CliError::DependencyError {
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

            return Err(CliError::CircularDependency {
                models: remaining.join(", "),
            }
            .into());
        }

        Ok(order)
    }

    pub fn get_model(&self, name: &str) -> Result<&ModelFile> {
        self.models
            .get(name)
            .ok_or_else(|| anyhow!("Model not found: {}", name))
    }

    pub fn models(&self) -> &HashMap<String, ModelFile> {
        &self.models
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
        use crate::config::{SourceColumn, SourceConfig, SourceSchema, SourceTable};

        let models = vec![make_model("A", vec!["source.events"])];

        let mut sources = HashMap::new();
        let mut tables = HashMap::new();
        tables.insert(
            "events".to_string(),
            SourceTable {
                description: String::new(),
                columns: vec![SourceColumn::new(
                    "id".to_string(),
                    "INTEGER".to_string(),
                    String::new(),
                )],
            },
        );
        sources.insert("source".to_string(), SourceSchema { tables });

        let source_config = SourceConfig {
            version: 1,
            sources,
        };

        let graph = DependencyGraph::build(models, Some(&source_config)).unwrap();
        assert!(graph.validate().is_ok());
    }
}
