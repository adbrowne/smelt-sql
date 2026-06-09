//! Generic value types and row generation driven by [`GeneratorSpec`].

use crate::config::{ColumnConfig, EntityConfig, FkCounts, GeneratorSpec, LinkedPoolConfig};
use crate::gen::Gen;
use crate::generators::{
    bool_with_prob, geometric, log_normal, one_of, uniform, uuid_gen, weighted_choice,
};
use chrono::NaiveDate;
use indexmap::IndexMap;
use rand::distributions::{Distribution, WeightedIndex};
use rand::RngCore;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::collections::HashMap;
use anyhow::{anyhow, Result};
use std::sync::Arc;

/// Row-scoped state passed into `apply_spec` and `generate_row`.
///
/// Bundles together the per-row pool samples, the dataset-level pool map,
/// and the row context values that were previously threaded as separate
/// parameters (`row_index`, `fk_counts`). An empty `RowContext` (no pools,
/// no samples) is the right thing to pass when generating entity-column
/// values — entity columns don't see linked pools in v1.
pub struct RowContext<'a> {
    pub row_index: usize,
    pub fk_counts: &'a FkCounts,
    /// Dataset-level map of pool name → pre-built `LinkedPool`.
    pub pools: &'a HashMap<String, Arc<LinkedPool>>,
    /// Per-row map of pool name → the pool-entry index drawn for this row.
    /// Populated once per row before column iteration.
    pub pool_samples: &'a PoolSamples<'a>,
}

/// Per-row map of pool name → the selected pool-entry index for this row.
///
/// Built once per row before column iteration; the same index is used for
/// every `linked_choice` column that references the same pool within that row.
pub type PoolSamples<'a> = HashMap<&'a str, usize>;

/// Build per-row pool samples: one uniform index per pool, drawn from `rng`.
///
/// The caller passes in the full pools map; this function returns a map of
/// pool name → sampled index. The index is in `[0, pool.len())`.
///
/// Uses `rand::distributions::Uniform` rather than modulo on a `u64` draw
/// so the sampling is unbiased for arbitrary pool sizes — `validate_config`
/// rejects `pool_size: 0`, so `pool.len()` is always positive here.
pub fn sample_pools<'a>(
    rng: &mut impl RngCore,
    pools: &'a HashMap<String, Arc<LinkedPool>>,
) -> PoolSamples<'a> {
    use rand::distributions::{Distribution, Uniform};
    pools
        .iter()
        .map(|(name, pool)| {
            let len = pool.len().max(1);
            let idx = Uniform::new(0, len).sample(rng);
            (name.as_str(), idx)
        })
        .collect()
}

/// A single generated value.
#[derive(Debug, Clone)]
pub enum GenericValue {
    Str(String),
    Int(i32),
    Float(f64),
    Bool(bool),
    Null,
    /// A pre-encoded JSON value (currently always a JSON object string,
    /// produced by `JsonObject` generators). Stored separately from `Str`
    /// so it can be embedded into a containing `json_object` without being
    /// re-escaped as a string literal. From the Parquet writer's
    /// perspective `JsonRaw` is interchangeable with `Str` (both produce
    /// a `Utf8` column).
    JsonRaw(String),
}

/// A pool of pre-generated entity rows (for sticky attributes).
pub struct EntityPool {
    /// One entry per entity; each entry is one value per entity column.
    pub rows: Vec<Vec<GenericValue>>,
}

impl EntityPool {
    pub fn new(seed: u64, count: usize, col_specs: &[ColumnConfig]) -> Result<Self> {
        let empty_fk = FkCounts::new();
        let empty_pools: HashMap<String, Arc<LinkedPool>> = HashMap::new();
        let empty_samples: PoolSamples<'_> = HashMap::new();
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut rows = Vec::with_capacity(count);
        for i in 0..count {
            let ctx = RowContext {
                row_index: i,
                fk_counts: &empty_fk,
                pools: &empty_pools,
                pool_samples: &empty_samples,
            };
            let row: Result<Vec<_>> = col_specs
                .iter()
                .map(|c| apply_spec(&mut rng, &c.generator, &ctx))
                .collect();
            rows.push(row?);
        }
        Ok(Self { rows })
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// A pre-computed joint-distribution pool of tuples drawn from [`LinkedPoolConfig`].
///
/// `rows[i]` is the i-th pool entry: a tuple of [`GenericValue`]s in the order the
/// shape's `fields:` were declared. `field_index` maps field name → position within
/// each tuple (built once from the first shape, since all shapes agree on field names
/// per spec invariant 7).
pub struct LinkedPool {
    pub rows: Vec<Vec<GenericValue>>,
    pub field_index: IndexMap<String, usize>,
}

impl LinkedPool {
    /// Build a linked pool from `cfg`, using `seed` as the RNG seed.
    ///
    /// Callers are responsible for deriving the seed. When `cfg.seed` is set,
    /// pass it directly; otherwise the caller derives a deterministic offset
    /// from the dataset seed (per `docs/specs/datagen.md` §Pool construction:
    /// `dataset_seed.wrapping_add(100 + linked_pool_index)`). Pool seeding is
    /// kept separate from row generation so that changing `num_rows` does
    /// not perturb pool contents.
    pub fn new(seed: u64, cfg: &LinkedPoolConfig, fk_counts: &FkCounts) -> Result<LinkedPool> {
        // Build the field_index from the first shape (all shapes agree on field names).
        let field_index: IndexMap<String, usize> = cfg.shapes[0]
            .fields
            .keys()
            .enumerate()
            .map(|(i, name)| (name.clone(), i))
            .collect();

        // Build a WeightedIndex over shapes by weight.
        let weights: Vec<f64> = cfg.shapes.iter().map(|s| s.weight).collect();
        let shape_dist = WeightedIndex::new(&weights)
            .expect("WeightedIndex construction failed (all weights must be > 0)");

        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let pool_size = cfg.pool_size;
        let mut rows: Vec<Vec<GenericValue>> = Vec::with_capacity(pool_size);

        // Pool construction never samples from other linked pools (spec invariant 8).
        let empty_pools: HashMap<String, Arc<LinkedPool>> = HashMap::new();
        let empty_samples: PoolSamples<'_> = HashMap::new();

        while rows.len() < pool_size {
            // Sample a shape by weight.
            let shape_idx = shape_dist.sample(&mut rng);
            let shape = &cfg.shapes[shape_idx];

            // Determine which fields are sticky (draw once per shape-draw).
            let sticky_set: std::collections::HashSet<&str> =
                shape.sticky.iter().map(String::as_str).collect();

            // Draw all sticky fields once, in fields: declaration order.
            let mut sticky_values: IndexMap<&str, GenericValue> = IndexMap::new();
            for (field_name, spec) in &shape.fields {
                if sticky_set.contains(field_name.as_str()) {
                    let ctx = RowContext {
                        row_index: rows.len(),
                        fk_counts,
                        pools: &empty_pools,
                        pool_samples: &empty_samples,
                    };
                    let val = apply_spec(&mut rng, spec, &ctx)?;
                    sticky_values.insert(field_name.as_str(), val);
                }
            }

            // Emit `emit` entries, redrawing non-sticky fields per entry.
            for _ in 0..shape.emit {
                let mut tuple = Vec::with_capacity(shape.fields.len());
                for (field_name, spec) in &shape.fields {
                    let val = if sticky_set.contains(field_name.as_str()) {
                        // Reuse the once-drawn sticky value.
                        sticky_values[field_name.as_str()].clone()
                    } else {
                        // Redraw for each emitted entry.
                        let ctx = RowContext {
                            row_index: rows.len(),
                            fk_counts,
                            pools: &empty_pools,
                            pool_samples: &empty_samples,
                        };
                        apply_spec(&mut rng, spec, &ctx)?
                    };
                    tuple.push(val);
                }
                rows.push(tuple);
                // Stop as soon as we reach pool_size (truncate overshoot).
                if rows.len() >= pool_size {
                    break;
                }
            }
        }

        Ok(LinkedPool { rows, field_index })
    }

    /// Look up the value of `field` in pool entry `row_idx`.
    ///
    /// Returns `None` if `field` is not declared in this pool or if `row_idx`
    /// is out of bounds.
    pub fn get(&self, row_idx: usize, field: &str) -> Option<&GenericValue> {
        let col_idx = *self.field_index.get(field)?;
        self.rows.get(row_idx)?.get(col_idx)
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
///
/// `ctx` carries the row index, FK counts, pool map, and per-row pool samples.
pub fn generate_row(
    rng: &mut impl RngCore,
    entity_col_specs: &[ColumnConfig],
    entity_row: Option<&[GenericValue]>,
    col_specs: &[ColumnConfig],
    partition_col: Option<(&str, &str)>,
    ctx: &RowContext<'_>,
) -> Result<Vec<(String, GenericValue)>> {
    let mut row = Vec::new();

    // Entity columns first
    if let Some(entity_values) = entity_row {
        for (spec, value) in entity_col_specs.iter().zip(entity_values.iter()) {
            row.push((spec.name.clone(), value.clone()));
        }
    }

    // Regular columns
    for col in col_specs {
        let value = apply_spec(rng, &col.generator, ctx)?;
        row.push((col.name.clone(), value));
    }

    // Partition column last (string constant for that partition slice)
    if let Some((col_name, date_str)) = partition_col {
        row.push((
            col_name.to_string(),
            GenericValue::Str(date_str.to_string()),
        ));
    }

    Ok(row)
}

/// Build an EntityPool and return a closure that samples from it.
pub fn make_entity_pool(seed: u64, num_rows: usize, entity_cfg: &EntityConfig) -> Result<EntityPool> {
    let count = ((num_rows as f64) * entity_cfg.pool_ratio).max(1.0) as usize;
    EntityPool::new(seed, count, &entity_cfg.columns)
}

/// Map a [`GeneratorSpec`] to a concrete [`GenericValue`] using `rng`.
///
/// `ctx` carries row-scoped state: row index, FK counts, the dataset-level
/// linked-pool map, and per-row pool samples (drawn once before column
/// iteration — see [`sample_pools`]).
pub fn apply_spec(
    rng: &mut impl RngCore,
    spec: &GeneratorSpec,
    ctx: &RowContext<'_>,
) -> Result<GenericValue> {
    match spec {
        GeneratorSpec::Uuid => {
            let id = uuid_gen().generate(rng);
            Ok(GenericValue::Str(id.to_string()))
        }
        GeneratorSpec::Constant { value } => match value {
            serde_yaml::Value::Number(n) if n.is_i64() => {
                Ok(GenericValue::Int(n.as_i64().unwrap() as i32))
            }
            serde_yaml::Value::Number(n) if n.is_u64() => {
                Ok(GenericValue::Int(n.as_u64().unwrap() as i32))
            }
            _ => Ok(GenericValue::Str(value.as_str().unwrap_or("").to_string())),
        },
        GeneratorSpec::WeightedChoice { values } => {
            let items: Vec<(String, f64)> = values.iter().map(|(k, v)| (k.clone(), *v)).collect();
            let choice = weighted_choice(items).generate(rng);
            Ok(GenericValue::Str(choice))
        }
        GeneratorSpec::OneOf { values } => {
            let choice = one_of(values.clone()).generate(rng);
            Ok(GenericValue::Str(choice))
        }
        GeneratorSpec::UniformInt { min, max } => {
            let v = uniform(*min..*max).generate(rng);
            Ok(GenericValue::Int(v))
        }
        GeneratorSpec::UniformFloat { min, max } => {
            let v = uniform(*min..*max).generate(rng);
            Ok(GenericValue::Float(v))
        }
        GeneratorSpec::LogNormal { median, sigma, max } => {
            let v = log_normal(*median, *sigma, *max).generate(rng);
            Ok(GenericValue::Int(v))
        }
        GeneratorSpec::Geometric { p, min } => {
            let v = geometric(*p).generate(rng);
            let v = if let Some(m) = min { v.max(*m) } else { v };
            Ok(GenericValue::Int(v))
        }
        GeneratorSpec::Bool { prob } => {
            let v = bool_with_prob(*prob).generate(rng);
            Ok(GenericValue::Bool(v))
        }
        GeneratorSpec::Optional { prob, inner } => {
            let r = (rng.next_u64() as f64) / (u64::MAX as f64);
            if r < *prob {
                apply_spec(rng, inner, ctx)
            } else {
                Ok(GenericValue::Null)
            }
        }
        GeneratorSpec::SequentialId => Ok(GenericValue::Int((ctx.row_index + 1) as i32)),
        GeneratorSpec::ForeignKey { dataset } => {
            let count = ctx.fk_counts.get(dataset).copied().unwrap_or(1) as u64;
            let id = (rng.next_u64() % count) + 1;
            Ok(GenericValue::Int(id as i32))
        }
        GeneratorSpec::Date { start, end } => {
            let start_date = NaiveDate::parse_from_str(start, "%Y-%m-%d")
                .unwrap_or_else(|_| NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
            let end_date = NaiveDate::parse_from_str(end, "%Y-%m-%d")
                .unwrap_or_else(|_| NaiveDate::from_ymd_opt(2024, 12, 31).unwrap());
            let days_range = (end_date - start_date).num_days().max(1) as u64;
            let offset = (rng.next_u64() % days_range) as i64;
            let date = start_date + chrono::Duration::days(offset);
            Ok(GenericValue::Str(date.format("%Y-%m-%d").to_string()))
        }
        GeneratorSpec::Timestamp { start, end } => {
            let start_dt = chrono::NaiveDateTime::parse_from_str(start, "%Y-%m-%dT%H:%M:%S")
                .unwrap_or_else(|_| {
                    NaiveDate::from_ymd_opt(2024, 1, 1)
                        .unwrap()
                        .and_hms_opt(0, 0, 0)
                        .unwrap()
                });
            let end_dt = chrono::NaiveDateTime::parse_from_str(end, "%Y-%m-%dT%H:%M:%S")
                .unwrap_or_else(|_| {
                    NaiveDate::from_ymd_opt(2024, 12, 31)
                        .unwrap()
                        .and_hms_opt(23, 59, 59)
                        .unwrap()
                });
            let secs_range = (end_dt - start_dt).num_seconds().max(1) as u64;
            let offset = (rng.next_u64() % secs_range) as i64;
            let ts = start_dt + chrono::Duration::seconds(offset);
            Ok(GenericValue::Str(ts.format("%Y-%m-%dT%H:%M:%S").to_string()))
        }
        GeneratorSpec::StringPattern { template } => {
            let result = apply_string_pattern(rng, template, ctx.row_index);
            Ok(GenericValue::Str(result))
        }
        GeneratorSpec::JsonObject { fields } => {
            let mut json = String::from("{");
            let mut first = true;
            for (key, inner_spec) in fields {
                if !first {
                    json.push(',');
                }
                first = false;
                json.push('"');
                escape_json_string(key, &mut json);
                json.push_str("\":");
                let inner_value = apply_spec(rng, inner_spec, ctx)?;
                append_generic_value_as_json(&inner_value, &mut json);
            }
            json.push('}');
            Ok(GenericValue::JsonRaw(json))
        }
        GeneratorSpec::LinkedChoice { pool, field } => {
            let idx = *ctx.pool_samples.get(pool.as_str()).ok_or_else(|| {
                anyhow!(
                    "linked_choice references pool {pool:?} but no sample was taken for this \
                     row — validate_config should have rejected this configuration before \
                     row generation"
                )
            })?;
            let pool_ref = ctx.pools.get(pool.as_str()).ok_or_else(|| {
                anyhow!(
                    "linked_choice references pool {pool:?} which was not built — \
                     validate_config should have rejected this configuration"
                )
            })?;
            pool_ref.get(idx, field).cloned().ok_or_else(|| {
                anyhow!("linked_choice field {field:?} missing in pool {pool:?} at index {idx}")
            })
        }
    }
}

/// Encode a [`GenericValue`] in JSON form, appending to `out`.
///
/// Per `docs/specs/datagen.md` §`json_object` encoding:
/// - `Int` / `Float` — unquoted JSON number.
/// - `Bool` — unquoted `true` / `false`.
/// - `Str` — JSON-escaped double-quoted string.
/// - `JsonRaw` — embedded as-is (nested `JsonObject` output).
/// - `Null` — unquoted `null` (an `optional` sub-generator that fired).
fn append_generic_value_as_json(value: &GenericValue, out: &mut String) {
    use std::fmt::Write;
    match value {
        GenericValue::Int(i) => {
            let _ = write!(out, "{}", i);
        }
        GenericValue::Float(f) => {
            let _ = write!(out, "{}", f);
        }
        GenericValue::Bool(true) => out.push_str("true"),
        GenericValue::Bool(false) => out.push_str("false"),
        GenericValue::Null => out.push_str("null"),
        GenericValue::JsonRaw(s) => out.push_str(s),
        GenericValue::Str(s) => {
            out.push('"');
            escape_json_string(s, out);
            out.push('"');
        }
    }
}

/// JSON-escape `s` and append to `out` (no surrounding quotes — the caller
/// owns those). Covers the JSON-standard escape set: `"`, `\`, backspace,
/// formfeed, newline, carriage return, tab. Other control characters below
/// 0x20 are emitted as `\u00XX`. UTF-8 above 0x7F passes through unescaped.
fn escape_json_string(s: &str, out: &mut String) {
    use std::fmt::Write;
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
}

/// Expand a string pattern template, replacing `{...}` placeholders.
///
/// Supported placeholders:
/// - `{sequential_id}` — row index + 1
/// - `{uuid}` — random UUID
/// - `{uniform_int:MIN-MAX}` — random integer in [MIN, MAX)
/// - `{one_of:a,b,c}` — random choice from comma-separated list
fn apply_string_pattern(rng: &mut impl RngCore, template: &str, row_index: usize) -> String {
    let mut result = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '{' {
            // Collect until closing '}'
            let mut placeholder = String::new();
            for inner in chars.by_ref() {
                if inner == '}' {
                    break;
                }
                placeholder.push(inner);
            }
            let expanded = expand_placeholder(rng, &placeholder, row_index);
            result.push_str(&expanded);
        } else {
            result.push(ch);
        }
    }

    result
}

fn expand_placeholder(rng: &mut impl RngCore, placeholder: &str, row_index: usize) -> String {
    if placeholder == "sequential_id" {
        return (row_index + 1).to_string();
    }
    if placeholder == "uuid" {
        return uuid_gen().generate(rng).to_string();
    }
    if let Some(args) = placeholder.strip_prefix("uniform_int:") {
        if let Some((min_s, max_s)) = args.split_once('-') {
            let min: i32 = min_s.trim().parse().unwrap_or(0);
            let max: i32 = max_s.trim().parse().unwrap_or(100);
            let range = (max - min).max(1) as u64;
            let v = min + (rng.next_u64() % range) as i32;
            return v.to_string();
        }
    }
    if let Some(args) = placeholder.strip_prefix("one_of:") {
        let items: Vec<&str> = args.split(',').map(|s| s.trim()).collect();
        if !items.is_empty() {
            let idx = rng.next_u64() as usize % items.len();
            return items[idx].to_string();
        }
    }
    // Unknown placeholder — return as-is with braces
    format!("{{{}}}", placeholder)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_string_pattern_sequential_id() {
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let result = apply_string_pattern(&mut rng, "user_{sequential_id}@example.com", 0);
        assert_eq!(result, "user_1@example.com");
        let result = apply_string_pattern(&mut rng, "user_{sequential_id}@example.com", 99);
        assert_eq!(result, "user_100@example.com");
    }

    #[test]
    fn test_string_pattern_uniform_int() {
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let result = apply_string_pattern(&mut rng, "SKU-{uniform_int:1000-9999}", 0);
        // Should start with SKU- and have a number
        assert!(result.starts_with("SKU-"));
        let num: i32 = result.strip_prefix("SKU-").unwrap().parse().unwrap();
        assert!((1000..9999).contains(&num));
    }

    #[test]
    fn test_string_pattern_one_of() {
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let result = apply_string_pattern(&mut rng, "{one_of:red,green,blue}", 0);
        assert!(
            result == "red" || result == "green" || result == "blue",
            "got: {}",
            result
        );
    }

    #[test]
    fn test_string_pattern_multiple_placeholders() {
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let result = apply_string_pattern(&mut rng, "{one_of:a,b}-{sequential_id}", 4);
        assert!(result.ends_with("-5"), "got: {}", result);
    }

    #[test]
    fn test_string_pattern_unknown_placeholder() {
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let result = apply_string_pattern(&mut rng, "hello {unknown} world", 0);
        assert_eq!(result, "hello {unknown} world");
    }

    fn empty_ctx(fk: &FkCounts) -> RowContext<'_> {
        static EMPTY_POOLS: std::sync::OnceLock<HashMap<String, Arc<LinkedPool>>> =
            std::sync::OnceLock::new();
        static EMPTY_SAMPLES: std::sync::OnceLock<PoolSamples<'static>> =
            std::sync::OnceLock::new();
        let _ = EMPTY_POOLS.get_or_init(HashMap::new);
        let _ = EMPTY_SAMPLES.get_or_init(HashMap::new);
        RowContext {
            row_index: 0,
            fk_counts: fk,
            pools: EMPTY_POOLS.get().unwrap(),
            pool_samples: EMPTY_SAMPLES.get().unwrap(),
        }
    }

    #[test]
    fn test_date_generator_range() {
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let fk = FkCounts::new();
        let spec = GeneratorSpec::Date {
            start: "2024-01-01".to_string(),
            end: "2024-01-10".to_string(),
        };
        let ctx = empty_ctx(&fk);
        for _ in 0..50 {
            if let GenericValue::Str(s) = apply_spec(&mut rng, &spec, &ctx).unwrap() {
                let date = NaiveDate::parse_from_str(&s, "%Y-%m-%d").unwrap();
                let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
                let end = NaiveDate::from_ymd_opt(2024, 1, 10).unwrap();
                assert!(date >= start && date < end, "date out of range: {}", s);
            } else {
                panic!("Expected Str variant");
            }
        }
    }

    #[test]
    fn test_timestamp_generator_range() {
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let fk = FkCounts::new();
        let spec = GeneratorSpec::Timestamp {
            start: "2024-01-01T00:00:00".to_string(),
            end: "2024-01-02T00:00:00".to_string(),
        };
        let ctx = empty_ctx(&fk);
        if let GenericValue::Str(s) = apply_spec(&mut rng, &spec, &ctx).unwrap() {
            assert!(s.starts_with("2024-01-01T"), "got: {}", s);
        } else {
            panic!("Expected Str variant");
        }
    }

    /// Regression for FINDINGS bug #4: the geometric generator must default to
    /// `min: 1` so specs requiring positive counts (e.g. order quantities)
    /// don't silently get zeros. Callers can still pass `min: 0` explicitly to
    /// opt back into the old behaviour — covered by
    /// `test_geometric_explicit_min_zero_respected`.
    #[test]
    fn test_geometric_default_min_is_one() {
        // Parse via YAML to exercise the same code path as real users — relying
        // on the serde `default` attribute, not in-Rust struct construction.
        let spec: GeneratorSpec = serde_yaml::from_str("type: geometric\np: 0.5\n")
            .expect("geometric spec without min should parse");
        let mut rng = ChaCha8Rng::seed_from_u64(7);
        let fk = FkCounts::new();
        let ctx = empty_ctx(&fk);
        let mut min_seen = i32::MAX;
        let mut zero_count = 0;
        for _ in 0..10_000 {
            if let GenericValue::Int(v) = apply_spec(&mut rng, &spec, &ctx).unwrap() {
                if v < min_seen {
                    min_seen = v;
                }
                if v == 0 {
                    zero_count += 1;
                }
            } else {
                panic!("Expected Int variant from geometric");
            }
        }
        assert!(
            min_seen >= 1,
            "default geometric must not emit values < 1 (saw {} zero(s), min={})",
            zero_count,
            min_seen,
        );
    }

    /// An explicit `min: 0` overrides the new default and restores
    /// "may emit zeros" behaviour, so existing configs that intentionally want
    /// zeros aren't broken.
    #[test]
    fn test_geometric_explicit_min_zero_respected() {
        let spec: GeneratorSpec = serde_yaml::from_str("type: geometric\np: 0.5\nmin: 0\n")
            .expect("geometric spec with explicit min: 0 should parse");
        let mut rng = ChaCha8Rng::seed_from_u64(7);
        let fk = FkCounts::new();
        let ctx = empty_ctx(&fk);
        let mut saw_zero = false;
        for _ in 0..10_000 {
            if let GenericValue::Int(v) = apply_spec(&mut rng, &spec, &ctx).unwrap() {
                if v == 0 {
                    saw_zero = true;
                    break;
                }
            }
        }
        assert!(
            saw_zero,
            "explicit min: 0 must allow zeros (none in 10k samples — default leaked through?)",
        );
    }

    /// Field iteration order matches YAML declaration order, and the emitted
    /// JSON string is byte-exact (no whitespace, no key reordering).
    #[test]
    fn test_json_object_emits_object_with_field_order() {
        let yaml = r#"
type: json_object
fields:
  a: { type: constant, value: 1 }
  b: { type: constant, value: "x" }
  c: { type: bool, prob: 0.0 }
"#;
        let spec: GeneratorSpec = serde_yaml::from_str(yaml).unwrap();
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let fk = FkCounts::new();
        let ctx = empty_ctx(&fk);
        let GenericValue::JsonRaw(json) = apply_spec(&mut rng, &spec, &ctx).unwrap() else {
            panic!("expected JsonRaw");
        };
        assert_eq!(json, r#"{"a":1,"b":"x","c":false}"#);
    }

    /// `optional { prob: 0.0, inner: ... }` always fires the null branch, and
    /// the field is still present in the emitted object as `"<key>": null`.
    /// This is the spec's "always-emit-the-field" rule.
    #[test]
    fn test_json_object_optional_field_emits_null_not_omitted() {
        let yaml = r#"
type: json_object
fields:
  always_null:
    type: optional
    prob: 0.0
    inner: { type: constant, value: "v" }
"#;
        let spec: GeneratorSpec = serde_yaml::from_str(yaml).unwrap();
        let mut rng = ChaCha8Rng::seed_from_u64(7);
        let fk = FkCounts::new();
        let ctx = empty_ctx(&fk);
        let GenericValue::JsonRaw(json) = apply_spec(&mut rng, &spec, &ctx).unwrap() else {
            panic!("expected JsonRaw");
        };
        assert_eq!(json, r#"{"always_null":null}"#);
    }

    /// A `json_object` field whose generator is itself `json_object` produces
    /// a nested object value, not a string-of-string.
    #[test]
    fn test_json_object_nested() {
        let yaml = r#"
type: json_object
fields:
  outer:
    type: json_object
    fields:
      inner: { type: constant, value: 1 }
"#;
        let spec: GeneratorSpec = serde_yaml::from_str(yaml).unwrap();
        let mut rng = ChaCha8Rng::seed_from_u64(7);
        let fk = FkCounts::new();
        let ctx = empty_ctx(&fk);
        let GenericValue::JsonRaw(json) = apply_spec(&mut rng, &spec, &ctx).unwrap() else {
            panic!("expected JsonRaw");
        };
        assert_eq!(json, r#"{"outer":{"inner":1}}"#);
    }

    /// `optional { inner: json_object }`, when the optional fires, produces a
    /// nested object value — not a string-encoded one. Regression test for
    /// the embed-raw-vs-escape-string distinction.
    #[test]
    fn test_json_object_optional_inner_json_object_embeds_raw() {
        let yaml = r#"
type: json_object
fields:
  meta:
    type: optional
    prob: 1.0
    inner:
      type: json_object
      fields:
        kind: { type: constant, value: "x" }
"#;
        let spec: GeneratorSpec = serde_yaml::from_str(yaml).unwrap();
        let mut rng = ChaCha8Rng::seed_from_u64(7);
        let fk = FkCounts::new();
        let ctx = empty_ctx(&fk);
        let GenericValue::JsonRaw(json) = apply_spec(&mut rng, &spec, &ctx).unwrap() else {
            panic!("expected JsonRaw");
        };
        assert_eq!(json, r#"{"meta":{"kind":"x"}}"#);
    }

    /// JSON-escape coverage: `"`, `\`, `\n`, `\r`, `\t`, `\b`, `\f`, and
    /// control characters `< 0x20` as `\uXXXX`. Non-ASCII passes through.
    #[test]
    fn test_json_object_string_escapes() {
        let mut buf = String::new();
        escape_json_string("\"\\\n\r\t\x08\x0c", &mut buf);
        assert_eq!(buf, r#"\"\\\n\r\t\b\f"#);

        let mut buf = String::new();
        escape_json_string("\x01\x1f", &mut buf);
        assert_eq!(buf, r"\u0001\u001f");

        let mut buf = String::new();
        escape_json_string("café — 日本語", &mut buf);
        assert_eq!(buf, "café — 日本語");
    }

    /// Same seed + same config + same row index → byte-identical JSON.
    #[test]
    fn test_json_object_deterministic() {
        let yaml = r#"
type: json_object
fields:
  a: { type: uniform_int, min: 0, max: 100 }
  b: { type: bool, prob: 0.5 }
  c:
    type: optional
    prob: 0.5
    inner: { type: one_of, values: [x, y, z] }
"#;
        let spec: GeneratorSpec = serde_yaml::from_str(yaml).unwrap();
        let fk = FkCounts::new();

        let mut rng_a = ChaCha8Rng::seed_from_u64(123);
        let mut rng_b = ChaCha8Rng::seed_from_u64(123);
        for i in 0..50 {
            let empty_pools: HashMap<String, Arc<LinkedPool>> = HashMap::new();
            let empty_samples: PoolSamples<'_> = HashMap::new();
            let ctx = RowContext {
                row_index: i,
                fk_counts: &fk,
                pools: &empty_pools,
                pool_samples: &empty_samples,
            };
            let GenericValue::JsonRaw(a) = apply_spec(&mut rng_a, &spec, &ctx).unwrap() else {
                unreachable!()
            };
            let GenericValue::JsonRaw(b) = apply_spec(&mut rng_b, &spec, &ctx).unwrap() else {
                unreachable!()
            };
            assert_eq!(a, b, "deterministic mismatch at row {i}");
        }
    }

    /// Swapping field positions changes the emitted values, proving
    /// sub-generators consume RNG state in YAML declaration order rather
    /// than some unspecified order.
    #[test]
    fn test_json_object_rng_consumption_order() {
        let yaml_ab = r#"
type: json_object
fields:
  a: { type: uniform_int, min: 0, max: 1000 }
  b: { type: uniform_int, min: 0, max: 1000 }
"#;
        let yaml_ba = r#"
type: json_object
fields:
  b: { type: uniform_int, min: 0, max: 1000 }
  a: { type: uniform_int, min: 0, max: 1000 }
"#;
        let spec_ab: GeneratorSpec = serde_yaml::from_str(yaml_ab).unwrap();
        let spec_ba: GeneratorSpec = serde_yaml::from_str(yaml_ba).unwrap();
        let fk = FkCounts::new();
        let ctx = empty_ctx(&fk);
        let mut rng_ab = ChaCha8Rng::seed_from_u64(99);
        let mut rng_ba = ChaCha8Rng::seed_from_u64(99);
        let GenericValue::JsonRaw(ab) = apply_spec(&mut rng_ab, &spec_ab, &ctx).unwrap() else {
            unreachable!()
        };
        let GenericValue::JsonRaw(ba) = apply_spec(&mut rng_ba, &spec_ba, &ctx).unwrap() else {
            unreachable!()
        };
        // Different field order → different field-to-value assignment.
        assert_ne!(ab, ba);
    }

    /// `Float` sub-generators emit unquoted JSON numbers.
    #[test]
    fn test_json_object_float_unquoted() {
        let yaml = r#"
type: json_object
fields:
  ratio: { type: uniform_float, min: 0.1, max: 0.9 }
"#;
        let spec: GeneratorSpec = serde_yaml::from_str(yaml).unwrap();
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        let fk = FkCounts::new();
        let ctx = empty_ctx(&fk);
        let GenericValue::JsonRaw(json) = apply_spec(&mut rng, &spec, &ctx).unwrap() else {
            unreachable!()
        };
        // The float lives between the `:` and `}`; assert it's not quoted.
        let value_part = &json["{\"ratio\":".len()..json.len() - 1];
        assert!(
            !value_part.starts_with('"'),
            "float value must be unquoted; got {json}"
        );
        let parsed: f64 = value_part.parse().expect("float value parses");
        assert!((0.1..0.9).contains(&parsed));
    }

    /// Keys with JSON-special characters are properly escaped on the key
    /// side as well as the value side. (Unusual in practice, but possible
    /// via the YAML quoted-key syntax.)
    #[test]
    fn test_json_object_key_escapes() {
        let yaml = r#"
type: json_object
fields:
  "weird\"key": { type: constant, value: 1 }
"#;
        let spec: GeneratorSpec = serde_yaml::from_str(yaml).unwrap();
        let mut rng = ChaCha8Rng::seed_from_u64(0);
        let fk = FkCounts::new();
        let ctx = empty_ctx(&fk);
        let GenericValue::JsonRaw(json) = apply_spec(&mut rng, &spec, &ctx).unwrap() else {
            unreachable!()
        };
        assert_eq!(json, r#"{"weird\"key":1}"#);
    }

    // ── Phase 3 TDD tests: LinkedPool construction ────────────────────────────

    use crate::config::{LinkedPoolConfig, ShapeConfig};
    use indexmap::IndexMap;

    /// Helper: build a minimal single-shape `LinkedPoolConfig`.
    fn make_single_shape_pool(
        pool_size: usize,
        emit: usize,
        sticky: Vec<String>,
        fields: IndexMap<String, GeneratorSpec>,
    ) -> LinkedPoolConfig {
        LinkedPoolConfig {
            name: "test_pool".to_string(),
            pool_size,
            seed: Some(42),
            shapes: vec![ShapeConfig {
                weight: 1.0,
                emit,
                sticky,
                fields,
            }],
        }
    }

    /// Helper: make fields with two FK generators.
    fn two_fk_fields() -> IndexMap<String, GeneratorSpec> {
        let mut fields = IndexMap::new();
        fields.insert(
            "device_id".to_string(),
            GeneratorSpec::ForeignKey {
                dataset: "devices".to_string(),
            },
        );
        fields.insert(
            "user_id".to_string(),
            GeneratorSpec::ForeignKey {
                dataset: "users".to_string(),
            },
        );
        fields
    }

    /// Helper: make fk_counts for devices and users.
    fn fk_counts_100() -> FkCounts {
        let mut fk = FkCounts::new();
        fk.insert("devices".to_string(), 100);
        fk.insert("users".to_string(), 100);
        fk
    }

    /// A pool with `pool_size: 1000` and shapes with various `emit:` values
    /// produces exactly 1000 entries.
    #[test]
    fn test_linked_pool_size_is_exact() {
        let cfg = make_single_shape_pool(1000, 3, vec![], two_fk_fields());
        let fk = fk_counts_100();
        let pool = LinkedPool::new(42, &cfg, &fk).unwrap();
        assert_eq!(pool.len(), 1000, "pool must have exactly pool_size entries");
    }

    /// Two builds with the same seed and config produce byte-identical pool
    /// contents (determinism guarantee).
    #[test]
    fn test_linked_pool_deterministic() {
        let cfg = make_single_shape_pool(500, 2, vec![], two_fk_fields());
        let fk = fk_counts_100();
        let pool_a = LinkedPool::new(99, &cfg, &fk).unwrap();
        let pool_b = LinkedPool::new(99, &cfg, &fk).unwrap();
        assert_eq!(pool_a.rows.len(), pool_b.rows.len());
        for (i, (ra, rb)) in pool_a.rows.iter().zip(pool_b.rows.iter()).enumerate() {
            for (j, (va, vb)) in ra.iter().zip(rb.iter()).enumerate() {
                match (va, vb) {
                    (GenericValue::Int(a), GenericValue::Int(b)) => {
                        assert_eq!(a, b, "row {i} field {j} differs");
                    }
                    (GenericValue::Str(a), GenericValue::Str(b)) => {
                        assert_eq!(a, b, "row {i} field {j} differs");
                    }
                    _ => panic!("unexpected variant combination at row {i} field {j}"),
                }
            }
        }
    }

    /// Pool RNG stream is isolated from anything else the dataset is doing
    /// with RNG state. Build pool A, exhaustively consume an unrelated
    /// `ChaCha8Rng` seeded with the same value (simulating a row generator
    /// burning through draws against the dataset seed), then build pool B
    /// with the same args. A and B must be byte-identical — proving pool
    /// construction does not share RNG state with any other stream and
    /// therefore changing `num_rows` (which affects only the row side) cannot
    /// perturb pool contents.
    #[test]
    fn test_linked_pool_seed_isolation() {
        let cfg = make_single_shape_pool(200, 1, vec![], two_fk_fields());
        let fk = fk_counts_100();
        let pool_a = LinkedPool::new(7, &cfg, &fk).unwrap();

        // Burn an unrelated RNG seeded with the same value. If pool
        // construction shared global state with any other ChaCha8 stream,
        // pool_b below would diverge from pool_a.
        let mut unrelated = ChaCha8Rng::seed_from_u64(7);
        for _ in 0..10_000 {
            let _ = unrelated.next_u64();
        }

        let pool_b = LinkedPool::new(7, &cfg, &fk).unwrap();
        assert_eq!(pool_a.rows.len(), pool_b.rows.len());
        for (i, (ra, rb)) in pool_a.rows.iter().zip(pool_b.rows.iter()).enumerate() {
            for (j, (va, vb)) in ra.iter().zip(rb.iter()).enumerate() {
                match (va, vb) {
                    (GenericValue::Int(a), GenericValue::Int(b)) => {
                        assert_eq!(a, b, "row {i} field {j} differs");
                    }
                    _ => panic!("variant mismatch at row {i} field {j}"),
                }
            }
        }
    }

    /// A single-shape pool with `emit: 1` produces N tuples whose fields are
    /// drawn independently per entry (no sticky fields means all are redrawn).
    #[test]
    fn test_linked_pool_emit_one_basic() {
        let mut fields = IndexMap::new();
        fields.insert(
            "device_id".to_string(),
            GeneratorSpec::ForeignKey {
                dataset: "devices".to_string(),
            },
        );
        let mut fk = FkCounts::new();
        fk.insert("devices".to_string(), 1000);
        let cfg = make_single_shape_pool(100, 1, vec![], fields);
        let pool = LinkedPool::new(42, &cfg, &fk).unwrap();
        assert_eq!(pool.len(), 100);
        // All device_id values must be in [1, 1000].
        for (i, row) in pool.rows.iter().enumerate() {
            match &row[0] {
                GenericValue::Int(v) => assert!(
                    *v >= 1 && *v <= 1000,
                    "row {i}: device_id={v} out of [1,1000]"
                ),
                other => panic!("row {i}: expected Int, got {:?}", other),
            }
        }
        // With 1000 candidates and 100 draws, expect multiple distinct values.
        let distinct: std::collections::HashSet<i32> = pool
            .rows
            .iter()
            .map(|r| match &r[0] {
                GenericValue::Int(v) => *v,
                _ => unreachable!(),
            })
            .collect();
        assert!(
            distinct.len() > 5,
            "expected diversity in emit:1 draws; only {} distinct",
            distinct.len()
        );
    }

    /// `emit: 2, sticky: [device_id]` — pairs share `device_id`, differ in `user_id`.
    #[test]
    fn test_linked_pool_emit_two_sticky() {
        let fk = fk_counts_100();
        let cfg = make_single_shape_pool(100, 2, vec!["device_id".to_string()], two_fk_fields());
        let pool = LinkedPool::new(42, &cfg, &fk).unwrap();
        assert_eq!(pool.len(), 100);
        let device_idx = *pool.field_index.get("device_id").unwrap();
        let user_idx = *pool.field_index.get("user_id").unwrap();

        // Check pairs: every consecutive pair (0,1), (2,3), ... shares device_id.
        let mut any_differing_user = false;
        for pair in pool.rows.chunks(2) {
            if pair.len() < 2 {
                break;
            }
            let (d0, d1) = match (&pair[0][device_idx], &pair[1][device_idx]) {
                (GenericValue::Int(a), GenericValue::Int(b)) => (a, b),
                _ => panic!("expected Int for device_id"),
            };
            assert_eq!(d0, d1, "sticky device_id must be shared within a pair");
            let (u0, u1) = match (&pair[0][user_idx], &pair[1][user_idx]) {
                (GenericValue::Int(a), GenericValue::Int(b)) => (a, b),
                _ => panic!("expected Int for user_id"),
            };
            if u0 != u1 {
                any_differing_user = true;
            }
        }
        assert!(
            any_differing_user,
            "non-sticky user_id should differ within at least one pair (almost surely)"
        );
    }

    /// `emit: 2, sticky: [user_id]` — pairs share `user_id`.
    #[test]
    fn test_linked_pool_emit_two_sticky_user() {
        let fk = fk_counts_100();
        let cfg = make_single_shape_pool(100, 2, vec!["user_id".to_string()], two_fk_fields());
        let pool = LinkedPool::new(42, &cfg, &fk).unwrap();
        assert_eq!(pool.len(), 100);
        let user_idx = *pool.field_index.get("user_id").unwrap();
        let device_idx = *pool.field_index.get("device_id").unwrap();

        let mut any_differing_device = false;
        for pair in pool.rows.chunks(2) {
            if pair.len() < 2 {
                break;
            }
            let (u0, u1) = match (&pair[0][user_idx], &pair[1][user_idx]) {
                (GenericValue::Int(a), GenericValue::Int(b)) => (a, b),
                _ => panic!("expected Int for user_id"),
            };
            assert_eq!(u0, u1, "sticky user_id must be shared within a pair");
            let (d0, d1) = match (&pair[0][device_idx], &pair[1][device_idx]) {
                (GenericValue::Int(a), GenericValue::Int(b)) => (a, b),
                _ => panic!("expected Int for device_id"),
            };
            if d0 != d1 {
                any_differing_device = true;
            }
        }
        assert!(
            any_differing_device,
            "non-sticky device_id should differ within at least one pair (almost surely)"
        );
    }

    /// Shape weight distribution: three shapes with weights [0.6, 0.3, 0.1] produce
    /// pool entries in those proportions within ±1 pp across 100_000 entries.
    #[test]
    fn test_linked_pool_shape_weight_distribution() {
        let make_const_field = |val: &str| -> IndexMap<String, GeneratorSpec> {
            let mut m = IndexMap::new();
            m.insert(
                "tag".to_string(),
                GeneratorSpec::Constant {
                    value: serde_yaml::Value::String(val.to_string()),
                },
            );
            m
        };
        let cfg = LinkedPoolConfig {
            name: "wt_pool".to_string(),
            pool_size: 100_000,
            seed: Some(1),
            shapes: vec![
                ShapeConfig {
                    weight: 0.6,
                    emit: 1,
                    sticky: vec![],
                    fields: make_const_field("A"),
                },
                ShapeConfig {
                    weight: 0.3,
                    emit: 1,
                    sticky: vec![],
                    fields: make_const_field("B"),
                },
                ShapeConfig {
                    weight: 0.1,
                    emit: 1,
                    sticky: vec![],
                    fields: make_const_field("C"),
                },
            ],
        };
        let pool = LinkedPool::new(1, &cfg, &FkCounts::new()).unwrap();
        assert_eq!(pool.len(), 100_000);

        let mut counts = std::collections::HashMap::new();
        for row in &pool.rows {
            if let GenericValue::Str(tag) = &row[0] {
                *counts.entry(tag.clone()).or_insert(0usize) += 1;
            }
        }
        let a_share = counts["A"] as f64 / 100_000.0;
        let b_share = counts["B"] as f64 / 100_000.0;
        let c_share = counts["C"] as f64 / 100_000.0;
        assert!(
            (a_share - 0.6).abs() < 0.01,
            "A share {a_share:.4} not within 1pp of 0.60"
        );
        assert!(
            (b_share - 0.3).abs() < 0.01,
            "B share {b_share:.4} not within 1pp of 0.30"
        );
        assert!(
            (c_share - 0.1).abs() < 0.01,
            "C share {c_share:.4} not within 1pp of 0.10"
        );
    }

    /// Weights [2.0, 1.0] (sum 3.0) produce the same distribution as [0.667, 0.333].
    /// WeightedIndex normalises automatically.
    #[test]
    fn test_linked_pool_normalises_weights() {
        let make_const_fields = |val: &str| -> IndexMap<String, GeneratorSpec> {
            let mut m = IndexMap::new();
            m.insert(
                "tag".to_string(),
                GeneratorSpec::Constant {
                    value: serde_yaml::Value::String(val.to_string()),
                },
            );
            m
        };
        let make_cfg = |w0: f64, w1: f64, seed: u64| LinkedPoolConfig {
            name: "norm_pool".to_string(),
            pool_size: 10_000,
            seed: Some(seed),
            shapes: vec![
                ShapeConfig {
                    weight: w0,
                    emit: 1,
                    sticky: vec![],
                    fields: make_const_fields("A"),
                },
                ShapeConfig {
                    weight: w1,
                    emit: 1,
                    sticky: vec![],
                    fields: make_const_fields("B"),
                },
            ],
        };
        // Both with the same seed should produce similar distributions.
        let pool_abs = LinkedPool::new(5, &make_cfg(2.0, 1.0, 5), &FkCounts::new()).unwrap();
        let pool_norm = LinkedPool::new(5, &make_cfg(0.6667, 0.3333, 5), &FkCounts::new()).unwrap();

        let count_a = |pool: &LinkedPool| -> usize {
            pool.rows
                .iter()
                .filter(|r| matches!(&r[0], GenericValue::Str(s) if s == "A"))
                .count()
        };
        let a_abs = count_a(&pool_abs) as f64 / 10_000.0;
        let a_norm = count_a(&pool_norm) as f64 / 10_000.0;
        // Both should be near 2/3 (~0.667); allow ±2 pp relative to each other.
        assert!(
            (a_abs - a_norm).abs() < 0.02,
            "abs weights ({a_abs:.4}) vs norm weights ({a_norm:.4}) differ by more than 2pp"
        );
    }

    /// Overshoot truncation: `pool_size: 100` with `emit: 30` produces exactly 100.
    /// The 4th shape draw (entries 90–119) is truncated at 100.
    #[test]
    fn test_linked_pool_truncates_overshoot() {
        let mut fields = IndexMap::new();
        fields.insert("v".to_string(), GeneratorSpec::SequentialId);
        let cfg = LinkedPoolConfig {
            name: "trunc_pool".to_string(),
            pool_size: 100,
            seed: Some(42),
            shapes: vec![ShapeConfig {
                weight: 1.0,
                emit: 30,
                sticky: vec![],
                fields,
            }],
        };
        let pool = LinkedPool::new(42, &cfg, &FkCounts::new()).unwrap();
        assert_eq!(
            pool.len(),
            100,
            "truncation must give exactly pool_size=100"
        );
    }

    /// Degenerate `pool_size: 1` works.
    #[test]
    fn test_linked_pool_handles_pool_size_one() {
        let cfg = make_single_shape_pool(1, 1, vec![], two_fk_fields());
        let fk = fk_counts_100();
        let pool = LinkedPool::new(42, &cfg, &fk).unwrap();
        assert_eq!(pool.len(), 1);
    }

    /// A `foreign_key { dataset: devices }` field with `FkCounts { devices: 100 }`
    /// produces values in [1, 100].
    #[test]
    fn test_linked_pool_foreign_key_resolves() {
        let mut fields = IndexMap::new();
        fields.insert(
            "device_id".to_string(),
            GeneratorSpec::ForeignKey {
                dataset: "devices".to_string(),
            },
        );
        let mut fk = FkCounts::new();
        fk.insert("devices".to_string(), 100);
        let cfg = make_single_shape_pool(500, 1, vec![], fields);
        let pool = LinkedPool::new(42, &cfg, &fk).unwrap();
        assert_eq!(pool.len(), 500);
        for (i, row) in pool.rows.iter().enumerate() {
            match &row[0] {
                GenericValue::Int(v) => assert!(
                    *v >= 1 && *v <= 100,
                    "row {i}: device_id={v} out of [1,100]"
                ),
                other => panic!("row {i}: expected Int, got {:?}", other),
            }
        }
    }

    /// `LinkedPool::get` resolves a field by name to the correct value.
    #[test]
    fn test_linked_pool_get_by_name() {
        let mut fields = IndexMap::new();
        fields.insert(
            "device_id".to_string(),
            GeneratorSpec::Constant {
                value: serde_yaml::Value::Number(serde_yaml::Number::from(7i64)),
            },
        );
        fields.insert(
            "user_id".to_string(),
            GeneratorSpec::Constant {
                value: serde_yaml::Value::Number(serde_yaml::Number::from(99i64)),
            },
        );
        let cfg = make_single_shape_pool(5, 1, vec![], fields);
        let pool = LinkedPool::new(1, &cfg, &FkCounts::new()).unwrap();
        // All constant so every row has device_id=7, user_id=99.
        for i in 0..5 {
            let dev = pool.get(i, "device_id").expect("device_id present");
            let usr = pool.get(i, "user_id").expect("user_id present");
            assert!(
                matches!(dev, GenericValue::Int(7)),
                "row {i}: device_id={:?}",
                dev
            );
            assert!(
                matches!(usr, GenericValue::Int(99)),
                "row {i}: user_id={:?}",
                usr
            );
        }
        assert!(pool.get(0, "missing").is_none());
    }
}
