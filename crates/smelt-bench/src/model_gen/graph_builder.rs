use rand::prelude::*;
use rand_chacha::ChaChaRng;

/// Configuration for generating the model dependency graph.
#[derive(Debug, Clone)]
pub struct GraphSpec {
    /// RNG seed for deterministic generation.
    pub seed: u64,
    /// Number of SQL models per layer (layers 1-4).
    pub sql_per_layer: usize,
    /// Number of Python models per layer (layers 1-4).
    pub python_per_layer: usize,
    /// Number of layers (excluding the source layer 0).
    pub num_layers: usize,
    /// Minimum dependencies per model (from previous layer).
    pub min_deps: usize,
    /// Maximum dependencies per model (from previous layer).
    pub max_deps: usize,
    /// Number of source tables in layer 0.
    pub num_sources: usize,
}

impl Default for GraphSpec {
    fn default() -> Self {
        Self {
            seed: 0xDEADBEEF,
            sql_per_layer: 250,
            python_per_layer: 250,
            num_layers: 4,
            min_deps: 1,
            max_deps: 3,
            num_sources: 20,
        }
    }
}

impl GraphSpec {
    /// Total number of models (excluding sources).
    pub fn total_models(&self) -> usize {
        (self.sql_per_layer + self.python_per_layer) * self.num_layers
    }

    /// Small spec for testing (20 models).
    pub fn small() -> Self {
        Self {
            seed: 0xDEADBEEF,
            sql_per_layer: 3,
            python_per_layer: 2,
            num_layers: 4,
            min_deps: 1,
            max_deps: 2,
            num_sources: 5,
        }
    }
}

/// Type of model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelType {
    Sql,
    Python,
}

/// Specification for a single model in the graph.
#[derive(Debug, Clone)]
pub struct ModelSpec {
    /// Model name (unique identifier).
    pub name: String,
    /// Layer number (1-4). Layer 0 is sources.
    pub layer: usize,
    /// Names of dependencies (models or sources from previous layer).
    pub dependencies: Vec<String>,
    /// Whether this is a SQL or Python model.
    pub model_type: ModelType,
}

/// Names of source tables.
pub const SOURCE_NAMES: &[&str] = &[
    "users",
    "events",
    "products",
    "orders",
    "sessions",
    "transactions",
    "page_views",
    "clicks",
    "signups",
    "payments",
    "refunds",
    "subscriptions",
    "invoices",
    "shipments",
    "reviews",
    "categories",
    "campaigns",
    "notifications",
    "logs",
    "errors",
];

/// Build a deterministic DAG of model specs.
///
/// Layer structure:
/// - Layer 0: Source tables (defined in sources.yml, not generated as models)
/// - Layer 1: Each refs 1 source. Simple staging/cleaning queries.
/// - Layer 2: Each refs 1-3 Layer 1 models. JOINs + aggregations.
/// - Layer 3: Each refs 2-3 Layer 2 models. Complex business logic.
/// - Layer 4: Each refs 1-3 Layer 3 models. Final/reporting.
pub fn build_graph(spec: &GraphSpec) -> Vec<ModelSpec> {
    let mut rng = ChaChaRng::seed_from_u64(spec.seed);
    let mut all_models = Vec::new();

    // Source names (layer 0)
    let source_names: Vec<String> = SOURCE_NAMES
        .iter()
        .take(spec.num_sources)
        .map(|s| s.to_string())
        .collect();

    // Track SQL-only names separately so SQL models never reference Python models.
    // Sources are discoverable by both SQL and Python models.
    let mut prev_sql_names = source_names.clone();
    let mut prev_all_names = source_names;

    for layer in 1..=spec.num_layers {
        let mut layer_sql_names = Vec::new();
        let mut layer_all_names = Vec::new();

        // Determine min/max deps for this layer
        let (min_deps, max_deps) = match layer {
            1 => (1, 1),                         // Layer 1: exactly 1 source
            2 => (1, spec.max_deps),             // Layer 2: 1-3
            3 => (2, spec.max_deps),             // Layer 3: 2-3
            _ => (spec.min_deps, spec.max_deps), // Layer 4+: configurable
        };

        let models_per_layer = spec.sql_per_layer + spec.python_per_layer;

        for i in 0..models_per_layer {
            let model_type = if i < spec.sql_per_layer {
                ModelType::Sql
            } else {
                ModelType::Python
            };

            let type_prefix = match model_type {
                ModelType::Sql => "sql",
                ModelType::Python => "py",
            };

            let name = format!("{}_l{}_{}", type_prefix, layer, i);

            // SQL models only reference other SQL models (+ sources).
            // Python models can reference any model type.
            let dep_pool = match model_type {
                ModelType::Sql => &prev_sql_names,
                ModelType::Python => &prev_all_names,
            };

            let num_deps = rng.gen_range(min_deps..=max_deps.min(dep_pool.len()));
            let deps = pick_unique(&mut rng, dep_pool, num_deps);

            all_models.push(ModelSpec {
                name: name.clone(),
                layer,
                dependencies: deps,
                model_type,
            });

            if model_type == ModelType::Sql {
                layer_sql_names.push(name.clone());
            }
            layer_all_names.push(name);
        }

        prev_sql_names = layer_sql_names;
        prev_all_names = layer_all_names;
    }

    all_models
}

/// Pick `count` unique items from `items` using the given RNG.
fn pick_unique(rng: &mut ChaChaRng, items: &[String], count: usize) -> Vec<String> {
    let count = count.min(items.len());
    let mut indices: Vec<usize> = (0..items.len()).collect();
    indices.shuffle(rng);
    indices.truncate(count);
    indices.into_iter().map(|i| items[i].clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    #[test]
    fn test_default_spec_generates_2000_models() {
        let spec = GraphSpec::default();
        let models = build_graph(&spec);
        assert_eq!(models.len(), 2000);
    }

    #[test]
    fn test_deterministic_generation() {
        let spec = GraphSpec::default();
        let models1 = build_graph(&spec);
        let models2 = build_graph(&spec);

        assert_eq!(models1.len(), models2.len());
        for (m1, m2) in models1.iter().zip(models2.iter()) {
            assert_eq!(m1.name, m2.name);
            assert_eq!(m1.dependencies, m2.dependencies);
        }
    }

    #[test]
    fn test_valid_dag() {
        let spec = GraphSpec::small();
        let models = build_graph(&spec);

        // Build a set of known names (sources + models)
        let source_names: HashSet<String> = SOURCE_NAMES
            .iter()
            .take(spec.num_sources)
            .map(|s| s.to_string())
            .collect();

        let mut known: HashSet<String> = source_names;
        let mut layer_map: HashMap<String, usize> = HashMap::new();

        for model in &models {
            // All dependencies must be in known set (defined before this model)
            for dep in &model.dependencies {
                assert!(
                    known.contains(dep),
                    "Model {} depends on unknown {}",
                    model.name,
                    dep
                );
            }

            // Dependencies must be from previous layers
            for dep in &model.dependencies {
                if let Some(&dep_layer) = layer_map.get(dep) {
                    assert!(
                        dep_layer < model.layer,
                        "Model {} (layer {}) depends on {} (layer {})",
                        model.name,
                        model.layer,
                        dep,
                        dep_layer
                    );
                }
                // If not in layer_map, it's a source (layer 0), which is fine
            }

            known.insert(model.name.clone());
            layer_map.insert(model.name.clone(), model.layer);
        }
    }

    #[test]
    fn test_small_spec() {
        let spec = GraphSpec::small();
        let models = build_graph(&spec);
        assert_eq!(models.len(), 20); // (3+2) * 4
    }

    #[test]
    fn test_model_types_distributed() {
        let spec = GraphSpec::small();
        let models = build_graph(&spec);

        let sql_count = models
            .iter()
            .filter(|m| m.model_type == ModelType::Sql)
            .count();
        let py_count = models
            .iter()
            .filter(|m| m.model_type == ModelType::Python)
            .count();

        assert_eq!(sql_count, 12); // 3 * 4
        assert_eq!(py_count, 8); // 2 * 4
    }
}
