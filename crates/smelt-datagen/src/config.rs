//! YAML-driven dataset configuration.

use arrow::datatypes::DataType;
use indexmap::IndexMap;
use serde::Deserialize;
use std::collections::HashMap;

/// Maps dataset name to its scaled row count. Used by `ForeignKey` to resolve
/// the target dimension size without storing any generated values.
pub type FkCounts = HashMap<String, usize>;

#[derive(Debug, Deserialize)]
pub struct DatagenConfig {
    pub seed: Option<u64>,
    pub scale_factor: Option<f64>,
    pub datasets: Vec<DatasetConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatasetConfig {
    pub name: String,
    pub output: String,
    pub num_rows: usize,
    pub seed: Option<u64>,
    pub partition: Option<PartitionConfig>,
    pub entity: Option<EntityConfig>,
    pub columns: Vec<ColumnConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PartitionConfig {
    pub column: String,
    /// Start date in "YYYY-MM-DD" format.
    pub start: String,
    pub days: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EntityConfig {
    /// Entity count = num_rows * pool_ratio.
    pub pool_ratio: f64,
    pub columns: Vec<ColumnConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ColumnConfig {
    pub name: String,
    pub generator: GeneratorSpec,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GeneratorSpec {
    Uuid,
    Constant {
        value: serde_yaml::Value,
    },
    WeightedChoice {
        values: IndexMap<String, f64>,
    },
    OneOf {
        values: Vec<String>,
    },
    UniformInt {
        min: i32,
        max: i32,
    },
    UniformFloat {
        min: f64,
        max: f64,
    },
    LogNormal {
        median: f64,
        sigma: f64,
        max: i32,
    },
    Geometric {
        p: f64,
    },
    Bool {
        prob: f64,
    },
    Optional {
        prob: f64,
        inner: Box<GeneratorSpec>,
    },
    SequentialId,
    ForeignKey {
        dataset: String,
    },
}

impl GeneratorSpec {
    /// Return the Arrow DataType produced by this generator.
    pub fn arrow_type(&self) -> DataType {
        match self {
            GeneratorSpec::Uuid => DataType::Utf8,
            GeneratorSpec::Constant { value } => match value {
                serde_yaml::Value::Number(n) if n.is_i64() || n.is_u64() => DataType::Int32,
                _ => DataType::Utf8,
            },
            GeneratorSpec::WeightedChoice { .. } => DataType::Utf8,
            GeneratorSpec::OneOf { .. } => DataType::Utf8,
            GeneratorSpec::UniformInt { .. } => DataType::Int32,
            GeneratorSpec::UniformFloat { .. } => DataType::Float64,
            GeneratorSpec::LogNormal { .. } => DataType::Int32,
            GeneratorSpec::Geometric { .. } => DataType::Int32,
            GeneratorSpec::Bool { .. } => DataType::Boolean,
            GeneratorSpec::Optional { inner, .. } => inner.arrow_type(),
            GeneratorSpec::SequentialId => DataType::Int32,
            GeneratorSpec::ForeignKey { .. } => DataType::Int32,
        }
    }

    /// Whether this generator can produce nulls (i.e. is Optional).
    pub fn is_nullable(&self) -> bool {
        matches!(self, GeneratorSpec::Optional { .. })
    }
}
