//! Generic value types and row generation driven by [`GeneratorSpec`].

use crate::config::{ColumnConfig, EntityConfig, FkCounts, GeneratorSpec};
use crate::gen::Gen;
use crate::generators::{
    bool_with_prob, geometric, log_normal, one_of, uniform, uuid_gen, weighted_choice,
};
use chrono::NaiveDate;
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
        let empty_fk = FkCounts::new();
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let rows = (0..count)
            .map(|i| {
                col_specs
                    .iter()
                    .map(|c| apply_spec(&mut rng, &c.generator, i, &empty_fk))
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
///
/// `row_index` is the global row index (0-based), used by `SequentialId`.
/// `fk_counts` maps dataset names to their scaled row counts, used by `ForeignKey`.
pub fn generate_row(
    rng: &mut impl RngCore,
    entity_col_specs: &[ColumnConfig],
    entity_row: Option<&[GenericValue]>,
    col_specs: &[ColumnConfig],
    partition_col: Option<(&str, &str)>,
    row_index: usize,
    fk_counts: &FkCounts,
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
        let value = apply_spec(rng, &col.generator, row_index, fk_counts);
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
pub fn apply_spec(
    rng: &mut impl RngCore,
    spec: &GeneratorSpec,
    row_index: usize,
    fk_counts: &FkCounts,
) -> GenericValue {
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
        GeneratorSpec::Geometric { p, min } => {
            let v = geometric(*p).generate(rng);
            let v = if let Some(m) = min { v.max(*m) } else { v };
            GenericValue::Int(v)
        }
        GeneratorSpec::Bool { prob } => {
            let v = bool_with_prob(*prob).generate(rng);
            GenericValue::Bool(v)
        }
        GeneratorSpec::Optional { prob, inner } => {
            let r = (rng.next_u64() as f64) / (u64::MAX as f64);
            if r < *prob {
                apply_spec(rng, inner, row_index, fk_counts)
            } else {
                GenericValue::Null
            }
        }
        GeneratorSpec::SequentialId => GenericValue::Int((row_index + 1) as i32),
        GeneratorSpec::ForeignKey { dataset } => {
            let count = fk_counts.get(dataset).copied().unwrap_or(1) as u64;
            let id = (rng.next_u64() % count) + 1;
            GenericValue::Int(id as i32)
        }
        GeneratorSpec::Date { start, end } => {
            let start_date = NaiveDate::parse_from_str(start, "%Y-%m-%d")
                .unwrap_or_else(|_| NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
            let end_date = NaiveDate::parse_from_str(end, "%Y-%m-%d")
                .unwrap_or_else(|_| NaiveDate::from_ymd_opt(2024, 12, 31).unwrap());
            let days_range = (end_date - start_date).num_days().max(1) as u64;
            let offset = (rng.next_u64() % days_range) as i64;
            let date = start_date + chrono::Duration::days(offset);
            GenericValue::Str(date.format("%Y-%m-%d").to_string())
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
            GenericValue::Str(ts.format("%Y-%m-%dT%H:%M:%S").to_string())
        }
        GeneratorSpec::StringPattern { template } => {
            let result = apply_string_pattern(rng, template, row_index);
            GenericValue::Str(result)
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

    #[test]
    fn test_date_generator_range() {
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let fk = FkCounts::new();
        let spec = GeneratorSpec::Date {
            start: "2024-01-01".to_string(),
            end: "2024-01-10".to_string(),
        };
        for _ in 0..50 {
            if let GenericValue::Str(s) = apply_spec(&mut rng, &spec, 0, &fk) {
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
        if let GenericValue::Str(s) = apply_spec(&mut rng, &spec, 0, &fk) {
            assert!(s.starts_with("2024-01-01T"), "got: {}", s);
        } else {
            panic!("Expected Str variant");
        }
    }
}
