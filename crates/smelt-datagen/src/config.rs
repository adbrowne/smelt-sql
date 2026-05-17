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
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
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
    /// Emit a JSON-encoded object as a single `Utf8` column. Each field's
    /// value is produced by an inner sub-generator; iteration order of
    /// `fields` is the order fields appear in the emitted JSON, and the order
    /// in which sub-generators consume RNG state. See `docs/specs/datagen.md`
    /// §`json_object` encoding.
    JsonObject {
        #[serde(deserialize_with = "deserialize_non_empty_json_fields")]
        fields: IndexMap<String, GeneratorSpec>,
    },
}

/// Reject `json_object` specs with an empty `fields:` map.
///
/// The spec rule is that the minimal valid `json_object` has at least one
/// field; an empty object provides no information and is almost always a
/// typo or copy-paste leftover.
fn deserialize_non_empty_json_fields<'de, D>(
    deserializer: D,
) -> Result<IndexMap<String, GeneratorSpec>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let fields = IndexMap::<String, GeneratorSpec>::deserialize(deserializer)?;
    if fields.is_empty() {
        return Err(D::Error::custom(
            "json_object `fields:` must contain at least one field",
        ));
    }
    Ok(fields)
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
            GeneratorSpec::JsonObject { .. } => DataType::Utf8,
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

    #[test]
    fn json_object_parses_minimal() {
        let yaml = "type: json_object\nfields:\n  k: { type: constant, value: \"v\" }\n";
        let spec: GeneratorSpec = serde_yaml::from_str(yaml).expect("minimal json_object parses");
        match spec {
            GeneratorSpec::JsonObject { fields } => {
                assert_eq!(fields.len(), 1);
                assert!(fields.contains_key("k"));
            }
            other => panic!("expected JsonObject, got {:?}", other),
        }
    }

    #[test]
    fn json_object_preserves_field_order() {
        // IndexMap should preserve the YAML declaration order. HashMap would
        // randomise per-process, making JSON output non-deterministic.
        let yaml = r#"
type: json_object
fields:
  a: { type: constant, value: 1 }
  b: { type: constant, value: 2 }
  c: { type: constant, value: 3 }
  d: { type: constant, value: 4 }
"#;
        let spec: GeneratorSpec = serde_yaml::from_str(yaml).expect("ordered fields parse");
        let GeneratorSpec::JsonObject { fields } = spec else {
            panic!("expected JsonObject");
        };
        let order: Vec<&str> = fields.keys().map(String::as_str).collect();
        assert_eq!(order, vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn json_object_rejects_empty_fields() {
        let yaml = "type: json_object\nfields: {}\n";
        let err = serde_yaml::from_str::<GeneratorSpec>(yaml)
            .expect_err("empty fields must be a parse error");
        let msg = err.to_string();
        assert!(
            msg.contains("at least one field") || msg.contains("fields"),
            "error must explain the rule; got: {msg}"
        );
    }

    /// Plan Phase 2 TDD item: `type: json_object` plus an unknown top-level
    /// key (e.g. `extra: 1`) must fail parse. Internally-tagged serde enums
    /// silently drop extras in variant payloads by default; the rejection
    /// here proves the deserialiser was given the equivalent of
    /// `deny_unknown_fields` for the `JsonObject` variant body.
    #[test]
    fn json_object_rejects_unknown_top_level_key() {
        let yaml = r#"
type: json_object
fields:
  k: { type: constant, value: "v" }
extra: 1
"#;
        let err = serde_yaml::from_str::<GeneratorSpec>(yaml)
            .expect_err("unknown top-level key must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("extra") || msg.contains("unknown field"),
            "error must name the offending field; got: {msg}"
        );
    }

    #[test]
    fn json_object_accepts_nested() {
        let yaml = r#"
type: json_object
fields:
  outer:
    type: json_object
    fields:
      inner: { type: constant, value: 1 }
"#;
        let spec: GeneratorSpec = serde_yaml::from_str(yaml).expect("nested json_object parses");
        let GeneratorSpec::JsonObject { fields } = spec else {
            panic!("expected outer JsonObject");
        };
        let outer = fields.get("outer").expect("outer field present");
        match outer {
            GeneratorSpec::JsonObject {
                fields: inner_fields,
            } => {
                assert!(inner_fields.contains_key("inner"));
            }
            other => panic!("expected nested JsonObject, got {:?}", other),
        }
    }

    #[test]
    fn json_object_arrow_type_is_utf8() {
        let mut fields = IndexMap::new();
        fields.insert(
            "k".to_string(),
            GeneratorSpec::Constant {
                value: serde_yaml::Value::String("v".to_string()),
            },
        );
        let spec = GeneratorSpec::JsonObject { fields };
        assert_eq!(spec.arrow_type(), DataType::Utf8);
    }

    #[test]
    fn json_object_is_not_nullable() {
        // The Utf8 column always has a value; nulls live *inside* the JSON
        // string, not at the Parquet column level.
        let mut fields = IndexMap::new();
        fields.insert(
            "k".to_string(),
            GeneratorSpec::Optional {
                prob: 0.0,
                inner: Box::new(GeneratorSpec::Constant {
                    value: serde_yaml::Value::String("v".to_string()),
                }),
            },
        );
        let spec = GeneratorSpec::JsonObject { fields };
        assert!(!spec.is_nullable());
    }
}
