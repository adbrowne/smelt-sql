//! YAML-driven dataset configuration.

use arrow::datatypes::DataType;
use indexmap::IndexMap;
use serde::Deserialize;
use std::collections::HashMap;

/// Maps dataset name to its scaled row count. Used by `ForeignKey` to resolve
/// the target dimension size without storing any generated values.
pub type FkCounts = HashMap<String, usize>;

// `deny_unknown_fields` on every top-level config struct: a typo'd or
// plausible-but-wrong key (e.g. `partition_by:` instead of `partition:`) used
// to silently parse and produce subtly wrong output. Failing parse with a
// clear "unknown field" error is the right ergonomic. (iter-4 issue #3.)

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatagenConfig {
    pub seed: Option<u64>,
    pub scale_factor: Option<f64>,
    pub datasets: Vec<DatasetConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct PartitionConfig {
    pub column: String,
    /// Start date in "YYYY-MM-DD" format.
    pub start: String,
    pub days: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntityConfig {
    /// Entity count = num_rows * pool_ratio.
    pub pool_ratio: f64,
    pub columns: Vec<ColumnConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
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
        /// Lower bound on the generated value. Defaults to `1` because the
        /// most common use of `geometric` is positive count-style data
        /// (quantities, retries, etc.) where zero is an invalid sentinel.
        /// Pass `min: 0` explicitly to opt back into "may emit zeros" — the
        /// raw geometric distribution starts at zero.
        #[serde(default = "default_geometric_min")]
        min: Option<i32>,
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
    Date {
        start: String,
        end: String,
    },
    Timestamp {
        start: String,
        end: String,
    },
    StringPattern {
        template: String,
    },
}

/// Default for [`GeneratorSpec::Geometric::min`] when the YAML omits it.
///
/// Returns `Some(1)` so the common case ("quantity never zero") Just Works;
/// callers that want zeros must now write `min: 0` explicitly. See the doc
/// comment on the field for the reasoning, and FINDINGS bug #4 /
/// `docs/plans/20260417-smelt-shop-0.3-followup.md` Phase B5 for history.
fn default_geometric_min() -> Option<i32> {
    Some(1)
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
            GeneratorSpec::Date { .. } => DataType::Utf8,
            GeneratorSpec::Timestamp { .. } => DataType::Utf8,
            GeneratorSpec::StringPattern { .. } => DataType::Utf8,
        }
    }

    /// Whether this generator can produce nulls (i.e. is Optional).
    pub fn is_nullable(&self) -> bool {
        matches!(self, GeneratorSpec::Optional { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// iter-4 issue #3 regression: a plausible-but-wrong YAML key like
    /// `partition_by:` (instead of `partition:`) used to silently parse —
    /// serde dropped the unknown field and the dataset was written
    /// un-partitioned. With `deny_unknown_fields` the parse must fail.
    #[test]
    fn dataset_config_rejects_unknown_top_level_field() {
        let yaml = r#"
name: page_events
output: data/page_events
num_rows: 1000
partition_by:
  column: event_date
  start: "2024-01-01"
  days: 5
columns:
  - name: id
    generator:
      type: uuid
"#;
        let err = serde_yaml::from_str::<DatasetConfig>(yaml)
            .expect_err("partition_by (typo for partition) must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("partition_by") || msg.contains("unknown field"),
            "error must name the offending field; got: {msg}"
        );
    }

    /// The correct `partition:` key still parses cleanly — the deny rule
    /// only catches genuine mistakes, not the documented schema.
    #[test]
    fn dataset_config_accepts_partition_key() {
        let yaml = r#"
name: page_events
output: data/page_events
num_rows: 1000
partition:
  column: event_date
  start: "2024-01-01"
  days: 5
columns:
  - name: id
    generator:
      type: uuid
"#;
        serde_yaml::from_str::<DatasetConfig>(yaml).expect("valid config must parse");
    }
}
