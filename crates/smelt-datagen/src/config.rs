//! YAML-driven dataset configuration.

use arrow::datatypes::DataType;
use indexmap::IndexMap;
use serde::Deserialize;
use std::collections::HashMap;

/// Maps dataset name to its scaled row count. Used by `ForeignKey` to resolve
/// the target dimension size without storing any generated values.
pub type FkCounts = HashMap<String, usize>;

// ── linked_pools: pre-computed joint-distribution pools ───────────────────────

/// A single shape template within a linked pool. Each draw of this shape
/// produces `emit` pool entries. Fields listed in `sticky` are drawn once per
/// shape draw and reused across all `emit` entries; all other fields are
/// redrawn per entry.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShapeConfig {
    #[serde(deserialize_with = "deserialize_positive_weight")]
    pub weight: f64,
    /// Number of pool entries produced per draw of this shape. Defaults to 1.
    #[serde(
        default = "default_shape_emit",
        deserialize_with = "deserialize_nonzero_emit"
    )]
    pub emit: usize,
    /// Fields drawn once per shape draw and repeated across emitted entries.
    /// Must be a subset of `fields`. Defaults to empty (all fields redrawn per entry).
    #[serde(default)]
    pub sticky: Vec<String>,
    #[serde(deserialize_with = "deserialize_shape_fields")]
    pub fields: IndexMap<String, GeneratorSpec>,
}

/// A named pool of pre-generated (field, …) tuples. Row-level `linked_choice`
/// generators draw from this pool.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkedPoolConfig {
    pub name: String,
    #[serde(deserialize_with = "deserialize_nonzero_pool_size")]
    pub pool_size: usize,
    /// Per-pool seed override. When absent the runtime derives one from the
    /// dataset seed and pool index.
    pub seed: Option<u64>,
    #[serde(deserialize_with = "deserialize_non_empty_shapes")]
    pub shapes: Vec<ShapeConfig>,
}

/// Reject a `shapes:` list that is empty — at least one shape is required.
/// Also validates:
/// - All shapes agree on field names (spec invariant 7).
/// - Each shape's `sticky:` is a subset of its `fields:` (spec §Semantics ¶4).
fn deserialize_non_empty_shapes<'de, D>(deserializer: D) -> Result<Vec<ShapeConfig>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let shapes = Vec::<ShapeConfig>::deserialize(deserializer)?;
    if shapes.is_empty() {
        return Err(D::Error::custom(
            "linked_pool `shapes:` must contain at least one shape",
        ));
    }
    // Validate sticky ⊆ fields for every shape.
    for (si, shape) in shapes.iter().enumerate() {
        let field_keys: Vec<&str> = shape.fields.keys().map(String::as_str).collect();
        for sticky_name in &shape.sticky {
            if !field_keys.contains(&sticky_name.as_str()) {
                return Err(D::Error::custom(format!(
                    "shape {si}: sticky field '{sticky_name}' is not declared in `fields:` \
                     (declared fields: {:?}); sticky fields must be a subset of fields",
                    field_keys
                )));
            }
        }
    }
    // Validate that all shapes agree on field names (spec invariant 7).
    let reference_keys: Vec<&str> = shapes[0].fields.keys().map(String::as_str).collect();
    for (i, shape) in shapes.iter().enumerate().skip(1) {
        let keys: Vec<&str> = shape.fields.keys().map(String::as_str).collect();
        if keys != reference_keys {
            return Err(D::Error::custom(format!(
                "all shapes in a linked_pool must declare the same field names; \
                 shape 0 has {:?} but shape {i} has {:?}",
                reference_keys, keys
            )));
        }
    }
    Ok(shapes)
}

/// Reject `weight: 0` and negative weights; the spec requires `weight > 0`.
fn deserialize_positive_weight<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let w = f64::deserialize(deserializer)?;
    if w <= 0.0 {
        return Err(D::Error::custom(format!(
            "shape `weight:` must be > 0 (got {w})"
        )));
    }
    Ok(w)
}

/// Reject `emit: 0`; the spec requires `emit ≥ 1`.
fn deserialize_nonzero_emit<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let e = usize::deserialize(deserializer)?;
    if e == 0 {
        return Err(D::Error::custom("shape `emit:` must be at least 1 (got 0)"));
    }
    Ok(e)
}

/// Reject `pool_size: 0`; the spec describes `pool_size:` as the exact
/// number of pool entries, and a zero-entry pool cannot satisfy any
/// `linked_choice` reference at row time. A `pool_size: 0` would slip past
/// pool construction (empty loop) and then panic the first time a
/// `linked_choice` column tried to look up a non-existent entry.
fn deserialize_nonzero_pool_size<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let n = usize::deserialize(deserializer)?;
    if n == 0 {
        return Err(D::Error::custom(
            "linked_pool `pool_size:` must be at least 1 (got 0)",
        ));
    }
    Ok(n)
}

fn default_shape_emit() -> usize {
    1
}

/// Deserialise a shape's `fields:` map, enforcing two invariants:
/// 1. `linked_choice` is not allowed inside `shapes[].fields` (spec invariant 8).
/// 2. `sticky:` must be a subset of the declared field names — enforced after
///    the parent `ShapeConfig` is fully deserialised in a post-parse validator
///    (see `validate_config`).
fn deserialize_shape_fields<'de, D>(
    deserializer: D,
) -> Result<IndexMap<String, GeneratorSpec>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let fields = IndexMap::<String, GeneratorSpec>::deserialize(deserializer)?;
    for (name, spec) in &fields {
        if contains_linked_choice(spec) {
            return Err(D::Error::custom(format!(
                "shape field `{name}` contains a `linked_choice` generator; \
                 `linked_choice` is not allowed inside `shapes[].fields` \
                 (pools cannot reference other pools — spec invariant 8)"
            )));
        }
    }
    Ok(fields)
}

/// Returns `true` if `spec` or any generator nested within it is a `LinkedChoice`.
/// Used to enforce the no-nested-pools rule (spec invariant 8) at parse time.
fn contains_linked_choice(spec: &GeneratorSpec) -> bool {
    match spec {
        GeneratorSpec::LinkedChoice { .. } => true,
        GeneratorSpec::Optional { inner, .. } => contains_linked_choice(inner),
        GeneratorSpec::JsonObject { fields } => fields.values().any(contains_linked_choice),
        _ => false,
    }
}

/// Post-parse configuration validator. Call this after `serde_yaml::from_str`
/// succeeds to enforce cross-dataset invariants that serde cannot check alone:
///
/// - Every `linked_choice` column's `pool:` names a pool declared in `linked_pools:`.
/// - Every `linked_choice` column's `field:` names a field present in that pool's shapes.
/// - Every shape's `sticky:` list is a subset of the shape's `fields:` keys.
///
/// This mirrors the existing FK reference-resolution pass in `main.rs::run_config`.
pub fn validate_config(config: &DatagenConfig) -> Result<(), String> {
    for dataset in &config.datasets {
        // Build a map from pool name → set of field names declared in that pool's shapes.
        // (All shapes agree on field names — enforced at parse time.)
        let pool_fields: HashMap<&str, Vec<&str>> = dataset
            .linked_pools
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(|p| {
                let field_names: Vec<&str> =
                    p.shapes[0].fields.keys().map(String::as_str).collect();
                (p.name.as_str(), field_names)
            })
            .collect();

        // Validate sticky fields are subsets of their shape's fields.
        if let Some(pools) = &dataset.linked_pools {
            for pool in pools {
                for (si, shape) in pool.shapes.iter().enumerate() {
                    let field_keys: Vec<&str> = shape.fields.keys().map(String::as_str).collect();
                    for sticky_name in &shape.sticky {
                        if !field_keys.contains(&sticky_name.as_str()) {
                            return Err(format!(
                                "pool '{}' shape {si}: sticky field '{sticky_name}' is not in \
                                 `fields:` (declared fields: {:?})",
                                pool.name, field_keys
                            ));
                        }
                    }
                }
            }
        }

        // Validate every linked_choice column reference.
        for col in &dataset.columns {
            validate_spec_linked_choice(&col.generator, &pool_fields, &col.name, &dataset.name)?;
        }
        // Also validate linked_choice inside entity columns (if any).
        if let Some(entity) = &dataset.entity {
            for col in &entity.columns {
                validate_spec_linked_choice(
                    &col.generator,
                    &pool_fields,
                    &col.name,
                    &dataset.name,
                )?;
            }
        }
    }
    Ok(())
}

/// Recursively walks `spec` and validates any `LinkedChoice` references against
/// `pool_fields` (pool name → field names).
fn validate_spec_linked_choice(
    spec: &GeneratorSpec,
    pool_fields: &HashMap<&str, Vec<&str>>,
    col_name: &str,
    dataset_name: &str,
) -> Result<(), String> {
    match spec {
        GeneratorSpec::LinkedChoice { pool, field } => match pool_fields.get(pool.as_str()) {
            None => {
                return Err(format!(
                    "dataset '{dataset_name}' column '{col_name}': \
                         linked_choice references pool '{pool}', \
                         but no pool named '{pool}' is declared in `linked_pools:`"
                ));
            }
            Some(field_names) => {
                if !field_names.contains(&field.as_str()) {
                    return Err(format!(
                        "dataset '{dataset_name}' column '{col_name}': \
                             linked_choice references field '{field}' in pool '{pool}', \
                             but '{field}' is not declared in that pool's shapes (fields: {:?})",
                        field_names
                    ));
                }
            }
        },
        GeneratorSpec::Optional { inner, .. } => {
            validate_spec_linked_choice(inner, pool_fields, col_name, dataset_name)?;
        }
        GeneratorSpec::JsonObject { fields } => {
            for inner_spec in fields.values() {
                validate_spec_linked_choice(inner_spec, pool_fields, col_name, dataset_name)?;
            }
        }
        _ => {}
    }
    Ok(())
}

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
    /// Pre-computed joint-distribution pools. Columns can reference these via
    /// `linked_choice` generators to draw correlated values.
    #[serde(default)]
    pub linked_pools: Option<Vec<LinkedPoolConfig>>,
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
    /// Draw one field from a pre-computed joint-distribution pool entry.
    ///
    /// Multiple `linked_choice` columns referencing the same `pool:` within a
    /// single row see the *same* pool tuple, producing correlated values (e.g.
    /// matched `(device_id, user_id)` pairs). The pool must be declared in
    /// `DatasetConfig::linked_pools`.
    ///
    /// `arrow_type()` returns a placeholder `Utf8` here; schema construction
    /// in `generic_parquet.rs` overrides this with the referenced field's
    /// actual generator type at write time (see Phase 4 implementation).
    /// `is_nullable()` returns `false` as a placeholder; the true nullability
    /// is also resolved at schema construction time against the pool definition.
    LinkedChoice {
        pool: String,
        field: String,
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
            // Placeholder — schema construction in generic_parquet.rs overrides this
            // with the referenced field's actual Arrow type. See Phase 4 implementation.
            GeneratorSpec::LinkedChoice { .. } => DataType::Utf8,
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

    // ── Phase 2 TDD tests: LinkedPoolConfig + ShapeConfig + LinkedChoice ──────

    /// A minimal `linked_pools:` block parses into `DatasetConfig.linked_pools`.
    #[test]
    fn linked_pool_parses_minimal() {
        let yaml = r#"
name: events
output: data/events
num_rows: 1000
linked_pools:
  - name: device_user
    pool_size: 500
    shapes:
      - weight: 1.0
        fields:
          device_id:
            type: foreign_key
            dataset: devices
          user_id:
            type: foreign_key
            dataset: users
columns:
  - name: id
    generator:
      type: uuid
"#;
        let cfg: DatasetConfig = serde_yaml::from_str(yaml).expect("minimal linked_pools parses");
        let pools = cfg.linked_pools.expect("linked_pools present");
        assert_eq!(pools.len(), 1);
        assert_eq!(pools[0].name, "device_user");
        assert_eq!(pools[0].pool_size, 500);
        assert_eq!(pools[0].shapes.len(), 1);
        assert_eq!(pools[0].shapes[0].fields.len(), 2);
    }

    /// `shapes:` declared in YAML order parse with that iteration order.
    #[test]
    fn linked_pool_preserves_shape_order() {
        let yaml = r#"
name: events
output: data/events
num_rows: 1000
linked_pools:
  - name: device_user
    pool_size: 100
    shapes:
      - weight: 0.6
        fields:
          tag: { type: constant, value: "A" }
      - weight: 0.3
        fields:
          tag: { type: constant, value: "B" }
      - weight: 0.1
        fields:
          tag: { type: constant, value: "C" }
columns:
  - name: id
    generator:
      type: uuid
"#;
        let cfg: DatasetConfig = serde_yaml::from_str(yaml).expect("shape order parses");
        let shapes = &cfg.linked_pools.unwrap()[0].shapes;
        assert_eq!(shapes.len(), 3);
        // Verify the constants to confirm ordering
        match &shapes[0].fields["tag"] {
            GeneratorSpec::Constant { value } => assert_eq!(value.as_str(), Some("A")),
            other => panic!("expected A constant, got {:?}", other),
        }
        match &shapes[1].fields["tag"] {
            GeneratorSpec::Constant { value } => assert_eq!(value.as_str(), Some("B")),
            other => panic!("expected B constant, got {:?}", other),
        }
        match &shapes[2].fields["tag"] {
            GeneratorSpec::Constant { value } => assert_eq!(value.as_str(), Some("C")),
            other => panic!("expected C constant, got {:?}", other),
        }
    }

    /// Within a shape, `fields:` iteration order matches YAML declaration order.
    #[test]
    fn linked_pool_preserves_field_order() {
        let yaml = r#"
name: events
output: data/events
num_rows: 1000
linked_pools:
  - name: device_user
    pool_size: 100
    shapes:
      - weight: 1.0
        fields:
          alpha: { type: constant, value: 1 }
          beta: { type: constant, value: 2 }
          gamma: { type: constant, value: 3 }
          delta: { type: constant, value: 4 }
columns:
  - name: id
    generator:
      type: uuid
"#;
        let cfg: DatasetConfig = serde_yaml::from_str(yaml).expect("field order parses");
        let fields = &cfg.linked_pools.unwrap()[0].shapes[0].fields;
        let order: Vec<&str> = fields.keys().map(String::as_str).collect();
        assert_eq!(order, vec!["alpha", "beta", "gamma", "delta"]);
    }

    /// `shapes: []` is a parse error.
    #[test]
    fn linked_pool_rejects_empty_shapes() {
        let yaml = r#"
name: events
output: data/events
num_rows: 1000
linked_pools:
  - name: device_user
    pool_size: 100
    shapes: []
columns:
  - name: id
    generator:
      type: uuid
"#;
        let err =
            serde_yaml::from_str::<DatasetConfig>(yaml).expect_err("empty shapes must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("shapes") || msg.contains("empty"),
            "error must name the offending field; got: {msg}"
        );
    }

    /// `weight: 0` is a parse error.
    #[test]
    fn shape_rejects_zero_weight() {
        let yaml = r#"
name: events
output: data/events
num_rows: 1000
linked_pools:
  - name: device_user
    pool_size: 100
    shapes:
      - weight: 0
        fields:
          device_id: { type: sequential_id }
columns:
  - name: id
    generator:
      type: uuid
"#;
        let err =
            serde_yaml::from_str::<DatasetConfig>(yaml).expect_err("weight: 0 must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("weight") || msg.contains("0") || msg.contains("positive"),
            "error must explain the rule; got: {msg}"
        );
    }

    /// `weight: -0.1` is a parse error.
    #[test]
    fn shape_rejects_negative_weight() {
        let yaml = r#"
name: events
output: data/events
num_rows: 1000
linked_pools:
  - name: device_user
    pool_size: 100
    shapes:
      - weight: -0.1
        fields:
          device_id: { type: sequential_id }
columns:
  - name: id
    generator:
      type: uuid
"#;
        let err = serde_yaml::from_str::<DatasetConfig>(yaml)
            .expect_err("negative weight must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("weight") || msg.contains("positive"),
            "error must explain the rule; got: {msg}"
        );
    }

    /// `emit: 0` is a parse error.
    #[test]
    fn shape_rejects_zero_emit() {
        let yaml = r#"
name: events
output: data/events
num_rows: 1000
linked_pools:
  - name: device_user
    pool_size: 100
    shapes:
      - weight: 1.0
        emit: 0
        fields:
          device_id: { type: sequential_id }
columns:
  - name: id
    generator:
      type: uuid
"#;
        let err =
            serde_yaml::from_str::<DatasetConfig>(yaml).expect_err("emit: 0 must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("emit") || msg.contains("0") || msg.contains("least"),
            "error must explain the rule; got: {msg}"
        );
    }

    /// `pool_size: 0` is a parse error. An empty pool cannot satisfy any
    /// `linked_choice` reference at row time; rejecting at parse time
    /// catches the misconfiguration before any panic-prone runtime path.
    #[test]
    fn linked_pool_rejects_zero_pool_size() {
        let yaml = r#"
name: events
output: data/events
num_rows: 1000
linked_pools:
  - name: device_user
    pool_size: 0
    shapes:
      - weight: 1.0
        fields:
          device_id: { type: sequential_id }
columns:
  - name: id
    generator:
      type: uuid
"#;
        let err =
            serde_yaml::from_str::<DatasetConfig>(yaml).expect_err("pool_size: 0 must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("pool_size") || msg.contains("at least 1") || msg.contains("0"),
            "error must explain the rule; got: {msg}"
        );
    }

    /// `emit:` omitted parses as `emit: 1`.
    #[test]
    fn shape_default_emit_is_one() {
        let yaml = r#"
name: events
output: data/events
num_rows: 1000
linked_pools:
  - name: device_user
    pool_size: 100
    shapes:
      - weight: 1.0
        fields:
          device_id: { type: sequential_id }
columns:
  - name: id
    generator:
      type: uuid
"#;
        let cfg: DatasetConfig = serde_yaml::from_str(yaml).expect("default emit parses");
        assert_eq!(cfg.linked_pools.unwrap()[0].shapes[0].emit, 1);
    }

    /// `sticky:` omitted parses as `sticky: []`.
    #[test]
    fn shape_default_sticky_is_empty() {
        let yaml = r#"
name: events
output: data/events
num_rows: 1000
linked_pools:
  - name: device_user
    pool_size: 100
    shapes:
      - weight: 1.0
        fields:
          device_id: { type: sequential_id }
columns:
  - name: id
    generator:
      type: uuid
"#;
        let cfg: DatasetConfig = serde_yaml::from_str(yaml).expect("default sticky parses");
        assert!(cfg.linked_pools.unwrap()[0].shapes[0].sticky.is_empty());
    }

    /// `sticky: [missing]` with `fields: { device_id, user_id }` is a parse error.
    #[test]
    fn shape_rejects_sticky_field_not_in_fields() {
        let yaml = r#"
name: events
output: data/events
num_rows: 1000
linked_pools:
  - name: device_user
    pool_size: 100
    shapes:
      - weight: 1.0
        sticky: [missing]
        fields:
          device_id: { type: sequential_id }
          user_id: { type: sequential_id }
columns:
  - name: id
    generator:
      type: uuid
"#;
        let err = serde_yaml::from_str::<DatasetConfig>(yaml)
            .expect_err("sticky field not in fields must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("missing") || msg.contains("sticky") || msg.contains("subset"),
            "error must name the offending field; got: {msg}"
        );
    }

    /// Two shapes with different `fields:` keys is a parse error.
    #[test]
    fn linked_pool_rejects_disagreeing_field_names_across_shapes() {
        let yaml = r#"
name: events
output: data/events
num_rows: 1000
linked_pools:
  - name: device_user
    pool_size: 100
    shapes:
      - weight: 0.5
        fields:
          device_id: { type: sequential_id }
          user_id: { type: sequential_id }
      - weight: 0.5
        fields:
          device_id: { type: sequential_id }
          session_id: { type: sequential_id }
columns:
  - name: id
    generator:
      type: uuid
"#;
        let err = serde_yaml::from_str::<DatasetConfig>(yaml)
            .expect_err("disagreeing field names must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("session_id")
                || msg.contains("user_id")
                || msg.contains("field")
                || msg.contains("agree"),
            "error must identify the disagreement; got: {msg}"
        );
    }

    /// A shape `fields:` containing `type: linked_choice` is a parse error (spec invariant 8).
    #[test]
    fn linked_pool_rejects_linked_choice_inside_shape_fields() {
        let yaml = r#"
name: events
output: data/events
num_rows: 1000
linked_pools:
  - name: outer_pool
    pool_size: 100
    shapes:
      - weight: 1.0
        fields:
          device_id:
            type: linked_choice
            pool: other_pool
            field: device_id
columns:
  - name: id
    generator:
      type: uuid
"#;
        let err = serde_yaml::from_str::<DatasetConfig>(yaml)
            .expect_err("linked_choice inside shape fields must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("linked_choice") || msg.contains("pool"),
            "error must mention linked_choice; got: {msg}"
        );
    }

    /// `pool_size: 10, shapes: [...], extra: 1` fails parse via `deny_unknown_fields`.
    #[test]
    fn linked_pool_rejects_unknown_top_level_keys() {
        let yaml = r#"
name: events
output: data/events
num_rows: 1000
linked_pools:
  - name: device_user
    pool_size: 100
    extra: 1
    shapes:
      - weight: 1.0
        fields:
          device_id: { type: sequential_id }
columns:
  - name: id
    generator:
      type: uuid
"#;
        let err = serde_yaml::from_str::<DatasetConfig>(yaml)
            .expect_err("unknown top-level key must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("extra") || msg.contains("unknown field"),
            "error must name the offending field; got: {msg}"
        );
    }

    /// `seed:` omitted leaves `seed: None`.
    #[test]
    fn linked_pool_optional_seed_default_is_none() {
        let yaml = r#"
name: events
output: data/events
num_rows: 1000
linked_pools:
  - name: device_user
    pool_size: 100
    shapes:
      - weight: 1.0
        fields:
          device_id: { type: sequential_id }
columns:
  - name: id
    generator:
      type: uuid
"#;
        let cfg: DatasetConfig = serde_yaml::from_str(yaml).expect("optional seed parses");
        assert!(cfg.linked_pools.unwrap()[0].seed.is_none());
    }

    /// A `linked_choice` generator parses into `GeneratorSpec::LinkedChoice { pool, field }`.
    #[test]
    fn linked_choice_variant_parses() {
        let yaml = "type: linked_choice\npool: device_user\nfield: device_id\n";
        let spec: GeneratorSpec = serde_yaml::from_str(yaml).expect("linked_choice parses");
        match spec {
            GeneratorSpec::LinkedChoice { pool, field } => {
                assert_eq!(pool, "device_user");
                assert_eq!(field, "device_id");
            }
            other => panic!("expected LinkedChoice, got {:?}", other),
        }
    }

    /// `arrow_type()` on a `LinkedChoice` variant returns placeholder `Utf8`.
    /// The actual type is resolved at schema construction time (Phase 4) using the
    /// referenced pool's field generator; see schema construction in generic_parquet.rs.
    #[test]
    fn linked_choice_arrow_type_resolved_lazily() {
        let spec = GeneratorSpec::LinkedChoice {
            pool: "device_user".to_string(),
            field: "device_id".to_string(),
        };
        // Placeholder Utf8 — schema construction will override with the referenced
        // field's actual type. See generic_parquet.rs `build_schema`.
        assert_eq!(spec.arrow_type(), DataType::Utf8);
    }

    /// `is_nullable()` on a `LinkedChoice` returns `false` for v1 — nullability
    /// is delegated to schema construction which has access to the pool definition.
    /// See generic_parquet.rs `build_schema` (Phase 4).
    #[test]
    fn linked_choice_is_not_nullable_unless_field_is_optional() {
        let spec = GeneratorSpec::LinkedChoice {
            pool: "device_user".to_string(),
            field: "device_id".to_string(),
        };
        assert!(!spec.is_nullable());
    }

    /// A column with `linked_choice` referencing an undeclared pool fails validation.
    #[test]
    fn dataset_config_rejects_linked_choice_pool_not_declared() {
        let yaml = r#"
name: events
output: data/events
num_rows: 1000
columns:
  - name: device_id
    generator:
      type: linked_choice
      pool: device_user
      field: device_id
"#;
        let cfg: DatasetConfig =
            serde_yaml::from_str(yaml).expect("serde parse succeeds (post-parse check)");
        let datagen_cfg = DatagenConfig {
            seed: None,
            scale_factor: None,
            datasets: vec![cfg],
        };
        let err = validate_config(&datagen_cfg).expect_err("undeclared pool must fail validation");
        assert!(
            err.contains("device_user") || err.contains("pool"),
            "error must name the undeclared pool; got: {err}"
        );
    }

    /// A column with `linked_choice` referencing an undeclared field in a pool fails validation.
    #[test]
    fn dataset_config_rejects_linked_choice_field_not_in_pool() {
        let yaml = r#"
name: events
output: data/events
num_rows: 1000
linked_pools:
  - name: device_user
    pool_size: 100
    shapes:
      - weight: 1.0
        fields:
          device_id: { type: sequential_id }
          user_id: { type: sequential_id }
columns:
  - name: session_id
    generator:
      type: linked_choice
      pool: device_user
      field: missing
"#;
        let cfg: DatasetConfig =
            serde_yaml::from_str(yaml).expect("serde parse succeeds (post-parse check)");
        let datagen_cfg = DatagenConfig {
            seed: None,
            scale_factor: None,
            datasets: vec![cfg],
        };
        let err = validate_config(&datagen_cfg).expect_err("undeclared field must fail validation");
        assert!(
            err.contains("missing") || err.contains("field"),
            "error must name the undeclared field; got: {err}"
        );
    }

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
