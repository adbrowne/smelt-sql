//! YAML line-scanning utilities for source rename edits.
//!
//! Pure functions that scan sources.yml content line-by-line to find
//! table keys and column names for rename operations. These avoid
//! full YAML parsing (which would destroy comments and formatting).

/// Find a source table key in sources.yml and produce the rename edit.
///
/// Scans for a table key under the given source name's `tables:` section.
/// Returns `(line_number, old_line, new_line)` or `None` if not found.
pub fn find_source_table_yaml_rename(
    yaml_content: &str,
    source_name: &str,
    old_table_name: &str,
    new_table_name: &str,
) -> Option<(u32, String, String)> {
    let mut in_source = false;
    let mut in_tables = false;
    for (i, line) in yaml_content.lines().enumerate() {
        let trimmed = line.trim();

        // Detect source name section (e.g., "  raw:")
        if !trimmed.starts_with('-') && trimmed.starts_with(&format!("{}:", source_name)) {
            in_source = true;
            in_tables = false;
            continue;
        }

        if in_source {
            if trimmed == "tables:" {
                in_tables = true;
                continue;
            }

            // A new top-level key resets context
            if !trimmed.is_empty()
                && !trimmed.starts_with('-')
                && !trimmed.starts_with('#')
                && !line.starts_with(' ')
            {
                in_source = false;
                in_tables = false;
                continue;
            }
        }

        if in_source && in_tables && trimmed.starts_with(&format!("{}:", old_table_name)) {
            let old_line = line.to_string();
            let new_line = line.replace(
                &format!("{}:", old_table_name),
                &format!("{}:", new_table_name),
            );
            return Some((i as u32, old_line, new_line));
        }
    }
    None
}

/// Find a column name entry in sources.yml and produce the rename edit.
///
/// Scans for `- name: old_column_name` entries.
/// Returns `(line_number, old_line, new_line)` or `None` if not found.
pub fn find_source_column_yaml_rename(
    yaml_content: &str,
    old_column_name: &str,
    new_column_name: &str,
) -> Option<(u32, String, String)> {
    for (i, line) in yaml_content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed == format!("- name: {}", old_column_name) {
            let old_line = line.to_string();
            let new_line = line.replace(
                &format!("- name: {}", old_column_name),
                &format!("- name: {}", new_column_name),
            );
            return Some((i as u32, old_line, new_line));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_source_table_yaml_rename_found() {
        let yaml = "\
sources:
  raw:
    tables:
      users:
        columns:
          - name: id
          - name: email
      orders:
        columns:
          - name: id";
        let result = find_source_table_yaml_rename(yaml, "raw", "users", "customers");
        assert!(result.is_some());
        let (line, old, new) = result.unwrap();
        assert_eq!(line, 3);
        assert!(old.contains("users:"));
        assert!(new.contains("customers:"));
    }

    #[test]
    fn test_find_source_table_yaml_rename_not_found() {
        let yaml = "\
sources:
  raw:
    tables:
      users:
        columns:
          - name: id";
        let result = find_source_table_yaml_rename(yaml, "raw", "nonexistent", "whatever");
        assert!(result.is_none());
    }

    #[test]
    fn test_find_source_column_yaml_rename_found() {
        let yaml = "\
sources:
  raw:
    tables:
      users:
        columns:
          - name: user_id
          - name: email";
        let result = find_source_column_yaml_rename(yaml, "user_id", "account_id");
        assert!(result.is_some());
        let (line, old, new) = result.unwrap();
        assert_eq!(line, 5);
        assert!(old.contains("- name: user_id"));
        assert!(new.contains("- name: account_id"));
    }

    #[test]
    fn test_find_source_column_yaml_rename_not_found() {
        let yaml = "\
sources:
  raw:
    tables:
      users:
        columns:
          - name: user_id";
        let result = find_source_column_yaml_rename(yaml, "nonexistent", "whatever");
        assert!(result.is_none());
    }
}
