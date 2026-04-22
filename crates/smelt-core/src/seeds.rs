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
/// since those are the ones referenced as `smelt.ref('seed_name')`.
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
}
