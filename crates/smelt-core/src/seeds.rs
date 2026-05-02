use smelt_types::DataType;
use std::path::{Path, PathBuf};

/// Information about a seed CSV file discovered in the project's seed directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedInfo {
    /// CSV filename without extension (used as the ref target name)
    pub name: String,
    /// Absolute path to the CSV file
    pub path: PathBuf,
    /// Column names and inferred types from the CSV headers + data
    pub columns: Vec<(String, DataType)>,
}

/// Discover seed CSV files in the project's seed directories and infer column types.
///
/// Seeds are CSV files in the configured seed directories. Top-level CSVs are
/// "target seeds" (schema = target schema). Subdirectory CSVs are "source seeds"
/// (schema = subdirectory name). This function discovers only top-level seeds
/// since those are the ones referenced as `smelt.models.seed_name`.
pub fn discover_seed_infos(project_dir: &Path, seed_paths: &[String]) -> Vec<SeedInfo> {
    let mut seeds = Vec::new();

    for seed_path in seed_paths {
        let seed_dir = project_dir.join(seed_path);
        if !seed_dir.exists() {
            continue;
        }

        let Ok(entries) = std::fs::read_dir(&seed_dir) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().is_some_and(|e| e == "csv") {
                let name = path
                    .file_stem()
                    .expect("CSV file always has a stem")
                    .to_string_lossy()
                    .into_owned();
                let columns = infer_csv_columns(&path);
                seeds.push(SeedInfo {
                    name,
                    path,
                    columns,
                });
            }
        }
    }

    seeds.sort_by(|a, b| a.name.cmp(&b.name));
    seeds
}

/// Parse CSV headers and infer column types from the first few data rows.
fn infer_csv_columns(path: &Path) -> Vec<(String, DataType)> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };

    let mut lines = content.lines();

    let headers_line = match lines.next() {
        Some(l) => l,
        None => return Vec::new(),
    };

    let headers: Vec<String> = headers_line
        .split(',')
        .map(|h| h.trim().to_string())
        .collect();

    // Collect first 10 data rows for type inference
    let data_rows: Vec<Vec<&str>> = lines
        .take(10)
        .map(|l| l.split(',').collect::<Vec<_>>())
        .collect();

    headers
        .into_iter()
        .enumerate()
        .map(|(i, header)| {
            let values: Vec<&str> = data_rows
                .iter()
                .filter_map(|row| row.get(i).copied())
                .filter(|v| !v.trim().is_empty())
                .collect();
            let dtype = infer_type_from_csv_values(&values);
            (header, dtype)
        })
        .collect()
}

/// Infer a SQL type from a sample of CSV string values.
///
/// The order of checks matters and intentionally matches DuckDB's
/// `read_csv_auto()` precedence: Boolean → Date → Timestamp → Integer →
/// Double → Text. Temporal checks come before Integer because we want a
/// column of `2025-01-01`-shaped values to be `Date`, not text-of-integers
/// — a bare `2025` would not match the `YYYY-MM-DD` shape, but ordering
/// the temporal checks first makes the policy obvious.
fn infer_type_from_csv_values(values: &[&str]) -> DataType {
    if values.is_empty() {
        return DataType::Text;
    }

    // Boolean: all values are true/false (case-insensitive)
    if values
        .iter()
        .all(|v| matches!(v.to_lowercase().as_str(), "true" | "false"))
    {
        return DataType::Boolean;
    }

    // Date: every value matches `YYYY-MM-DD` (4-digit year, 1-12 month,
    // 1-31 day). Matches DuckDB's `read_csv_auto()` DATE recognition for
    // ISO-formatted dates.
    if values.iter().all(|v| looks_like_date(v.trim())) {
        return DataType::Date;
    }

    // Timestamp: every value matches `YYYY-MM-DD HH:MM:SS` (optionally
    // with fractional seconds). The compile-time inferencer never emits
    // `with_timezone: true` because the ISO-without-zone shape we accept
    // here has no timezone information.
    if values.iter().all(|v| looks_like_timestamp(v.trim())) {
        return DataType::Timestamp {
            with_timezone: false,
        };
    }

    // Integer: all values parse as i64
    if values.iter().all(|v| v.parse::<i64>().is_ok()) {
        return DataType::Integer;
    }

    // Double: all values parse as f64
    if values.iter().all(|v| v.parse::<f64>().is_ok()) {
        return DataType::Double;
    }

    // Default to Text
    DataType::Text
}

/// `true` when `s` is shaped like `YYYY-MM-DD` with plausible field
/// ranges. Does not validate calendar correctness (Feb 30 passes); the
/// goal is to match what DuckDB's `read_csv_auto()` types as `DATE` for
/// the columns smelt's compile-time inferencer cares about.
fn looks_like_date(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 {
        return false;
    }
    parse_fixed_uint(parts[0], 4)
        .filter(|y| (1000..=9999).contains(y))
        .and(parse_fixed_uint(parts[1], 2).filter(|m| (1..=12).contains(m)))
        .and(parse_fixed_uint(parts[2], 2).filter(|d| (1..=31).contains(d)))
        .is_some()
}

/// `true` when `s` is shaped like `YYYY-MM-DD HH:MM:SS` (optionally with a
/// fractional-seconds tail like `.123`). Mirrors `looks_like_date`'s
/// permissive range checks: the goal is shape-recognition, not calendar
/// validation.
fn looks_like_timestamp(s: &str) -> bool {
    let (date_part, time_part) = match s.split_once(' ') {
        Some(parts) => parts,
        None => return false,
    };
    if !looks_like_date(date_part) {
        return false;
    }
    // Strip optional fractional-seconds tail (".123", ".123456", etc.).
    let time_core = time_part
        .split_once('.')
        .map(|(h, _)| h)
        .unwrap_or(time_part);
    let time_parts: Vec<&str> = time_core.split(':').collect();
    if time_parts.len() != 3 {
        return false;
    }
    parse_fixed_uint(time_parts[0], 2)
        .filter(|h| *h <= 23)
        .and(parse_fixed_uint(time_parts[1], 2).filter(|m| *m <= 59))
        .and(parse_fixed_uint(time_parts[2], 2).filter(|sec| *sec <= 59))
        .is_some()
}

/// Parse `s` as a non-negative integer, requiring `expected_len` ASCII
/// digits and nothing else. Returns `None` on length mismatch, leading
/// sign, embedded whitespace, or non-digit characters.
fn parse_fixed_uint(s: &str, expected_len: usize) -> Option<u32> {
    if s.len() != expected_len || !s.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    s.parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_discover_seed_infos_basic() {
        let tmp = TempDir::new().unwrap();
        let seeds_dir = tmp.path().join("seeds");
        fs::create_dir_all(&seeds_dir).unwrap();
        fs::write(
            seeds_dir.join("order_statuses.csv"),
            "status_code,status_label,is_terminal,is_successful\ncompleted,Completed,true,true\ncancelled,Cancelled,true,false\n",
        ).unwrap();

        let seeds = discover_seed_infos(tmp.path(), &["seeds".to_string()]);
        assert_eq!(seeds.len(), 1);
        assert_eq!(seeds[0].name, "order_statuses");
        assert_eq!(seeds[0].columns.len(), 4);

        let col_map: std::collections::HashMap<_, _> = seeds[0].columns.iter().cloned().collect();
        assert_eq!(col_map["status_code"], DataType::Text);
        assert_eq!(col_map["status_label"], DataType::Text);
        assert_eq!(col_map["is_terminal"], DataType::Boolean);
        assert_eq!(col_map["is_successful"], DataType::Boolean);
    }

    #[test]
    fn test_infer_numeric_types() {
        let tmp = TempDir::new().unwrap();
        let seeds_dir = tmp.path().join("seeds");
        fs::create_dir_all(&seeds_dir).unwrap();
        fs::write(
            seeds_dir.join("numbers.csv"),
            "id,score,ratio\n1,100,0.5\n2,200,1.5\n",
        )
        .unwrap();

        let seeds = discover_seed_infos(tmp.path(), &["seeds".to_string()]);
        let col_map: std::collections::HashMap<_, _> = seeds[0].columns.iter().cloned().collect();
        assert_eq!(col_map["id"], DataType::Integer);
        assert_eq!(col_map["score"], DataType::Integer);
        assert_eq!(col_map["ratio"], DataType::Double);
    }

    #[test]
    fn test_discover_no_seeds_dir() {
        let tmp = TempDir::new().unwrap();
        let seeds = discover_seed_infos(tmp.path(), &["seeds".to_string()]);
        assert!(seeds.is_empty());
    }

    #[test]
    fn test_seed_date_column_infers_as_date() {
        // TB-2 — a seed CSV column shaped like YYYY-MM-DD must infer as
        // `DataType::Date`, matching DuckDB's `read_csv_auto()` behaviour.
        let tmp = TempDir::new().unwrap();
        let seeds_dir = tmp.path().join("seeds");
        fs::create_dir_all(&seeds_dir).unwrap();
        fs::write(
            seeds_dir.join("users.csv"),
            "user_id,user_name,signup_date\n1,Alice,2025-01-01\n2,Bob,2025-01-02\n3,Charlie,2025-01-03\n",
        )
        .unwrap();

        let seeds = discover_seed_infos(tmp.path(), &["seeds".to_string()]);
        assert_eq!(seeds.len(), 1);
        let col_map: std::collections::HashMap<_, _> = seeds[0].columns.iter().cloned().collect();
        assert_eq!(col_map["user_id"], DataType::Integer);
        assert_eq!(col_map["user_name"], DataType::Text);
        assert_eq!(col_map["signup_date"], DataType::Date);
    }

    #[test]
    fn test_seed_timestamp_column_infers_as_timestamp() {
        // TB-2 — a seed CSV column shaped like YYYY-MM-DD HH:MM:SS must
        // infer as `DataType::Timestamp { with_timezone: false }`.
        let tmp = TempDir::new().unwrap();
        let seeds_dir = tmp.path().join("seeds");
        fs::create_dir_all(&seeds_dir).unwrap();
        fs::write(
            seeds_dir.join("events.csv"),
            "event_id,event_type,event_timestamp\n1,login,2025-01-10 08:00:00\n2,page_view,2025-01-10 08:05:00\n3,logout,2025-01-10 09:00:00\n",
        )
        .unwrap();

        let seeds = discover_seed_infos(tmp.path(), &["seeds".to_string()]);
        assert_eq!(seeds.len(), 1);
        let col_map: std::collections::HashMap<_, _> = seeds[0].columns.iter().cloned().collect();
        assert_eq!(col_map["event_id"], DataType::Integer);
        assert_eq!(col_map["event_type"], DataType::Text);
        assert_eq!(
            col_map["event_timestamp"],
            DataType::Timestamp {
                with_timezone: false
            }
        );
    }

    #[test]
    fn test_seed_text_column_infers_as_text() {
        // Regression guard — free-form strings still infer as Text after
        // the temporal inferencers are added. Strings that are *almost*
        // dates but not actually dates (out-of-range fields, wrong shape)
        // must remain Text rather than be coerced into Date/Timestamp.
        let tmp = TempDir::new().unwrap();
        let seeds_dir = tmp.path().join("seeds");
        fs::create_dir_all(&seeds_dir).unwrap();
        fs::write(
            seeds_dir.join("notes.csv"),
            "note_id,body,almost_date\n1,hello world,2025-13-01\n2,another note,not-a-date\n3,third,2025-01-32\n",
        )
        .unwrap();

        let seeds = discover_seed_infos(tmp.path(), &["seeds".to_string()]);
        assert_eq!(seeds.len(), 1);
        let col_map: std::collections::HashMap<_, _> = seeds[0].columns.iter().cloned().collect();
        assert_eq!(col_map["note_id"], DataType::Integer);
        assert_eq!(col_map["body"], DataType::Text);
        // Out-of-range month/day → not a Date; falls back to Text.
        assert_eq!(col_map["almost_date"], DataType::Text);
    }

    #[test]
    fn test_seed_t_separator_timestamp_falls_back_to_text() {
        // Spec promise (seeds.md Semantics §5): the compile-time inferencer
        // recognises `YYYY-MM-DD HH:MM:SS` (space separator) only. ISO-8601
        // `T`-separated timestamps fall back to Text and require an explicit
        // CAST in a downstream model.
        let tmp = TempDir::new().unwrap();
        let seeds_dir = tmp.path().join("seeds");
        fs::create_dir_all(&seeds_dir).unwrap();
        fs::write(
            seeds_dir.join("events.csv"),
            "event_id,iso_timestamp\n1,2025-01-10T08:00:00\n2,2025-01-10T08:05:00\n",
        )
        .unwrap();

        let seeds = discover_seed_infos(tmp.path(), &["seeds".to_string()]);
        let col_map: std::collections::HashMap<_, _> = seeds[0].columns.iter().cloned().collect();
        assert_eq!(col_map["iso_timestamp"], DataType::Text);
    }

    #[test]
    fn test_seed_tz_suffix_timestamp_falls_back_to_text() {
        // Spec promise (seeds.md Semantics §5): the compile-time inferencer
        // never emits TIMESTAMP WITH TIME ZONE. Columns containing zone
        // information (Z suffix, +00 / -05 offset, named zone) fall back to
        // Text.
        let tmp = TempDir::new().unwrap();
        let seeds_dir = tmp.path().join("seeds");
        fs::create_dir_all(&seeds_dir).unwrap();
        fs::write(
            seeds_dir.join("events.csv"),
            "event_id,zoned_ts,offset_ts\n1,2025-01-10 08:00:00Z,2025-01-10 08:00:00+00\n2,2025-01-10 08:05:00Z,2025-01-10 08:05:00-05\n",
        )
        .unwrap();

        let seeds = discover_seed_infos(tmp.path(), &["seeds".to_string()]);
        let col_map: std::collections::HashMap<_, _> = seeds[0].columns.iter().cloned().collect();
        assert_eq!(col_map["zoned_ts"], DataType::Text);
        assert_eq!(col_map["offset_ts"], DataType::Text);
    }
}
