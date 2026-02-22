//! Generic value types and row generation driven by [`GeneratorSpec`].

use crate::config::{ColumnConfig, EntityConfig, GeneratorSpec};
use crate::gen::Gen;
use crate::generators::{
    bool_with_prob, geometric, log_normal, one_of, uniform, uuid_gen, weighted_choice,
};
use rand::RngCore;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

/// A single generated value.
#[derive(Debug, Clone)]
pub enum GenericValue {
    Str(String),
    Int(i32),
    Float(f64),
    Bool(bool),
    Null,
}

/// A pool of pre-generated entity rows (for sticky attributes).
pub struct EntityPool {
    /// One entry per entity; each entry is one value per entity column.
    pub rows: Vec<Vec<GenericValue>>,
}

impl EntityPool {
    pub fn new(seed: u64, count: usize, col_specs: &[ColumnConfig]) -> Self {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let rows = (0..count)
            .map(|_| {
                col_specs
                    .iter()
                    .map(|c| apply_spec(&mut rng, &c.generator))
                    .collect()
            })
            .collect();
        Self { rows }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// Generate one row as a list of `(column_name, value)` pairs.
///
/// Column order:
/// 1. entity columns (if `entity_row` is provided)
/// 2. regular columns from `col_specs`
/// 3. partition column (if provided)
pub fn generate_row(
    rng: &mut impl RngCore,
    entity_col_specs: &[ColumnConfig],
    entity_row: Option<&[GenericValue]>,
    col_specs: &[ColumnConfig],
    partition_col: Option<(&str, &str)>,
) -> Vec<(String, GenericValue)> {
    let mut row = Vec::new();

    // Entity columns first
    if let Some(entity_values) = entity_row {
        for (spec, value) in entity_col_specs.iter().zip(entity_values.iter()) {
            row.push((spec.name.clone(), value.clone()));
        }
    }

    // Regular columns
    for col in col_specs {
        let value = apply_spec(rng, &col.generator);
        row.push((col.name.clone(), value));
    }

    // Partition column last (string constant for that partition slice)
    if let Some((col_name, date_str)) = partition_col {
        row.push((
            col_name.to_string(),
            GenericValue::Str(date_str.to_string()),
        ));
    }

    row
}

/// Build an EntityPool and return a closure that samples from it.
pub fn make_entity_pool(seed: u64, num_rows: usize, entity_cfg: &EntityConfig) -> EntityPool {
    let count = ((num_rows as f64) * entity_cfg.pool_ratio).max(1.0) as usize;
    EntityPool::new(seed, count, &entity_cfg.columns)
}

/// Map a [`GeneratorSpec`] to a concrete [`GenericValue`] using `rng`.
pub fn apply_spec(rng: &mut impl RngCore, spec: &GeneratorSpec) -> GenericValue {
    match spec {
        GeneratorSpec::Uuid => {
            let id = uuid_gen().generate(rng);
            GenericValue::Str(id.to_string())
        }
        GeneratorSpec::Constant { value } => match value {
            serde_yaml::Value::Number(n) if n.is_i64() => {
                GenericValue::Int(n.as_i64().unwrap() as i32)
            }
            serde_yaml::Value::Number(n) if n.is_u64() => {
                GenericValue::Int(n.as_u64().unwrap() as i32)
            }
            _ => GenericValue::Str(value.as_str().unwrap_or("").to_string()),
        },
        GeneratorSpec::WeightedChoice { values } => {
            let items: Vec<(String, f64)> = values.iter().map(|(k, v)| (k.clone(), *v)).collect();
            let choice = weighted_choice(items).generate(rng);
            GenericValue::Str(choice)
        }
        GeneratorSpec::OneOf { values } => {
            let choice = one_of(values.clone()).generate(rng);
            GenericValue::Str(choice)
        }
        GeneratorSpec::UniformInt { min, max } => {
            let v = uniform(*min..*max).generate(rng);
            GenericValue::Int(v)
        }
        GeneratorSpec::UniformFloat { min, max } => {
            let v = uniform(*min..*max).generate(rng);
            GenericValue::Float(v)
        }
        GeneratorSpec::LogNormal { median, sigma, max } => {
            let v = log_normal(*median, *sigma, *max).generate(rng);
            GenericValue::Int(v)
        }
        GeneratorSpec::Geometric { p } => {
            let v = geometric(*p).generate(rng);
            GenericValue::Int(v)
        }
        GeneratorSpec::Bool { prob } => {
            let v = bool_with_prob(*prob).generate(rng);
            GenericValue::Bool(v)
        }
        GeneratorSpec::Optional { prob, inner } => {
            let r = (rng.next_u64() as f64) / (u64::MAX as f64);
            if r < *prob {
                apply_spec(rng, inner)
            } else {
                GenericValue::Null
            }
        }
    }
}
