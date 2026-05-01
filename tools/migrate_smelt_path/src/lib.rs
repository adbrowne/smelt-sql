//! Migration tool: rewrite legacy `smelt.ref(...)`, `smelt.source(...)`, and
//! `smelt.fn.*` calls to the unified `smelt.<path>` syntax.
//!
//! ## Migration rules
//!
//! - `smelt.ref('name')` → `smelt.<path-to-model>` (workspace-aware lookup)
//! - `smelt.ref("name")` → `smelt.<path-to-model>` (double quotes)
//! - `smelt.source('a.b')` → `smelt.sources.a.b`
//! - `smelt.fn.name(` → `smelt.functions.name(`
//! - `smelt.fn.core.safe_divide(` → `smelt.functions.core.safe_divide(`
//!
//! Comment lines (lines whose first non-whitespace characters are `--`) are
//! NOT rewritten.
//!
//! ## Workspace-aware ref resolution
//!
//! `smelt.ref('name')` is resolved by scanning model SQL files in the
//! workspace to build a name→path-tuple map.  Each SQL file under a
//! configured model path contributes its path tuple, which is the
//! workspace-relative directory components joined by `.` plus the model name.
//! For example, `models/staging/stg_events.sql` → `["models", "staging",
//! "stg_events"]` → `smelt.models.staging.stg_events`.
//!
//! Multi-model files (those with `--- name: foo ---` delimiters) also contribute
//! their declared names under the same directory prefix.
//!
//! If a ref cannot be resolved (e.g. the model is a Python `.py` file not
//! in the SQL scan), the tool falls back to `smelt.models.<name>`.

use anyhow::Result;
use regex::{Captures, Regex};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Workspace model index
// ---------------------------------------------------------------------------

/// Map from bare model name to its full path segments (post-`smelt` prefix).
/// E.g. `"stg_events"` → `["models", "staging", "stg_events"]`.
#[derive(Default)]
pub struct ModelIndex {
    /// name → dot-joined path (e.g. `"models.staging.stg_events"`)
    pub map: HashMap<String, String>,
}

impl ModelIndex {
    /// Build a model index by scanning a workspace root.
    ///
    /// `model_path_dirs` is the list of model directories relative to `root`
    /// (e.g. `["models"]`).  Pass an empty slice to default to `["models"]`.
    ///
    /// CSV files in `seeds/` (relative to `root`) are also indexed under the
    /// `seeds.<name>` prefix so that `smelt.ref('order_statuses')` migrates to
    /// `smelt.seeds.order_statuses` rather than `smelt.models.order_statuses`.
    pub fn from_workspace(root: &Path, model_path_dirs: &[&str]) -> Result<Self> {
        let dirs: &[&str] = if model_path_dirs.is_empty() {
            &["models"]
        } else {
            model_path_dirs
        };

        let mut index = Self::default();

        // Index SQL model files.
        for dir in dirs {
            let search = root.join(dir);
            if !search.exists() {
                continue;
            }
            for entry in walkdir::WalkDir::new(&search)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) != Some("sql") {
                    continue;
                }
                // Compute path tuple relative to workspace root.
                let rel = p.strip_prefix(root).unwrap_or(p);
                let parent_segments: Vec<String> = rel
                    .parent()
                    .unwrap_or(Path::new(""))
                    .components()
                    .filter_map(|c| match c {
                        std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                        _ => None,
                    })
                    .collect();
                let prefix = parent_segments.join(".");

                // Collect model names from this file.
                let names = model_names_in_file(p)?;
                for name in names {
                    let full_path = if prefix.is_empty() {
                        name.clone()
                    } else {
                        format!("{}.{}", prefix, name)
                    };
                    index.map.insert(name, full_path);
                }
            }
        }

        // Index CSV seed files under `seeds/`.
        let seeds_dir = root.join("seeds");
        if seeds_dir.exists() {
            for entry in walkdir::WalkDir::new(&seeds_dir)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) != Some("csv") {
                    continue;
                }
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    index
                        .map
                        .insert(stem.to_string(), format!("seeds.{}", stem));
                }
            }
        }

        Ok(index)
    }

    /// Look up the full `smelt.<path>` for a model name.
    /// Falls back to `smelt.models.<name>` if not found.
    pub fn resolve(&self, name: &str) -> String {
        if let Some(path) = self.map.get(name) {
            format!("smelt.{}", path)
        } else {
            format!("smelt.models.{}", name)
        }
    }
}

/// Extract model names declared in a SQL file.
///
/// Supports:
/// - Single-model file: stem of the filename (`stg_events.sql` → `stg_events`),
///   possibly overridden by a `name:` in single-section frontmatter.
/// - Multi-model file: `--- name: foo ---` delimiter lines → `foo`.
fn model_names_in_file(path: &Path) -> Result<Vec<String>> {
    let content = std::fs::read_to_string(path)?;
    let names = extract_model_names(&content, path);
    Ok(names)
}

/// Pure helper: extract model names from file content + path.
pub fn extract_model_names(content: &str, path: &Path) -> Vec<String> {
    // Detect multi-model files by `--- name: <name> ---` delimiter pattern.
    let multi_re = Regex::new(r"^---\s+name:\s+(\S+)\s+---\s*$").unwrap();
    let mut multi_names: Vec<String> = Vec::new();
    for line in content.lines() {
        if let Some(caps) = multi_re.captures(line) {
            multi_names.push(caps[1].to_string());
        }
    }
    if !multi_names.is_empty() {
        return multi_names;
    }

    // Single-model file: look for `name:` in single-block YAML frontmatter.
    let name_in_fm = extract_name_from_frontmatter(content);
    if let Some(n) = name_in_fm {
        return vec![n];
    }

    // Fall back to file stem.
    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
        return vec![stem.to_string()];
    }

    Vec::new()
}

/// Extract `name:` value from a single-block `--- ... ---` frontmatter.
fn extract_name_from_frontmatter(content: &str) -> Option<String> {
    let name_re = Regex::new(r"(?m)^\s*name:\s+(\S+)\s*$").unwrap();
    // Find the first `---` block.
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }
    // Find closing `---`.
    let rest = &trimmed[3..];
    let end = rest.find("---")?;
    let fm = &rest[..end];
    name_re.captures(fm).map(|c| c[1].to_string())
}

// ---------------------------------------------------------------------------
// Content rewriting
// ---------------------------------------------------------------------------

/// Apply all migration rewrites to a single string of SQL content.
/// Lines that begin with `--` (after trimming leading whitespace) are skipped.
///
/// Uses a flat `smelt.models.<name>` fallback for `smelt.ref(...)` since
/// the workspace context is not available here.  Prefer
/// [`rewrite_content_with_index`] for accurate subdirectory paths.
pub fn rewrite_content(input: &str) -> String {
    rewrite_content_with_index(input, None)
}

/// Apply all migration rewrites, optionally using a [`ModelIndex`] to resolve
/// the correct path for `smelt.ref(...)` calls.
pub fn rewrite_content_with_index(input: &str, index: Option<&ModelIndex>) -> String {
    use std::sync::OnceLock;

    static RE_REF: OnceLock<Regex> = OnceLock::new();
    static RE_SOURCE: OnceLock<Regex> = OnceLock::new();
    static RE_FN: OnceLock<Regex> = OnceLock::new();

    let re_ref = RE_REF.get_or_init(|| {
        // Match smelt.ref('name') or smelt.ref("name")
        Regex::new("smelt\\.ref\\(['\"]([^'\"]+)['\"]\\)").unwrap()
    });
    let re_source = RE_SOURCE.get_or_init(|| {
        // Match smelt.source('a.b') or smelt.source("a.b")
        Regex::new("smelt\\.source\\(['\"]([^'\"]+)['\"]\\)").unwrap()
    });
    let re_fn = RE_FN.get_or_init(|| {
        // Matches `smelt.fn.` prefix
        Regex::new(r"smelt\.fn\.").unwrap()
    });

    let mut output = String::with_capacity(input.len());

    for line in input.split('\n') {
        // Check if this is a comment line — skip rewriting if so.
        let trimmed = line.trim_start();
        if trimmed.starts_with("--") {
            output.push_str(line);
            output.push('\n');
            continue;
        }

        // Apply rewrites in order.
        let line = re_ref.replace_all(line, |caps: &Captures| {
            let name = &caps[1];
            if let Some(idx) = index {
                idx.resolve(name)
            } else {
                format!("smelt.models.{}", name)
            }
        });
        let line = re_source.replace_all(&line, |caps: &Captures| {
            // `a.b` → `smelt.sources.a.b`  (dots already present)
            format!("smelt.sources.{}", &caps[1])
        });
        let line = re_fn.replace_all(&line, "smelt.functions.");

        output.push_str(&line);
        output.push('\n');
    }

    // The split/join above always appends a trailing '\n'.  If the original
    // input did NOT end with a newline we trim the extra one we added.
    if !input.ends_with('\n') && output.ends_with('\n') {
        output.pop();
    }

    output
}

// ---------------------------------------------------------------------------
// File and directory migration
// ---------------------------------------------------------------------------

/// Migrate a single file in-place.  Returns `true` if the file was modified.
pub fn migrate_file(path: &Path) -> Result<bool> {
    migrate_file_with_index(path, None)
}

/// Migrate a single file in-place using an optional [`ModelIndex`].
pub fn migrate_file_with_index(path: &Path, index: Option<&ModelIndex>) -> Result<bool> {
    let original = std::fs::read_to_string(path)?;
    let rewritten = rewrite_content_with_index(&original, index);
    if rewritten == original {
        return Ok(false);
    }
    std::fs::write(path, &rewritten)?;
    Ok(true)
}

/// Walk `dir` recursively and migrate every `.sql` file found.
/// Returns the number of files that were modified.
///
/// This is a workspace-unaware migration (uses `smelt.models.<name>` fallback).
/// Use [`migrate_workspace`] for accurate subdirectory paths.
pub fn migrate_directory(dir: &Path) -> Result<usize> {
    let mut count = 0;
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) == Some("sql") && migrate_file(p)? {
            count += 1;
        }
    }
    Ok(count)
}

/// Migrate all `.sql` files in the workspace rooted at `root`.
///
/// First builds a [`ModelIndex`] for `ref` resolution, then rewrites every
/// SQL file under each of `model_path_dirs` (defaults to `["models"]`).
/// Also migrates function files under `functions/`.
///
/// Returns the number of files modified.
pub fn migrate_workspace(root: &Path, model_path_dirs: &[&str]) -> Result<usize> {
    let dirs: Vec<&str> = if model_path_dirs.is_empty() {
        vec!["models"]
    } else {
        model_path_dirs.to_vec()
    };

    let index = ModelIndex::from_workspace(root, &dirs)?;
    let mut count = 0;

    // Also scan functions/ directory for smelt.ref() calls (rare but possible).
    let all_dirs: Vec<PathBuf> = dirs
        .iter()
        .map(|d| root.join(d))
        .chain(std::iter::once(root.join("functions")))
        .collect();

    for dir in &all_dirs {
        if !dir.exists() {
            continue;
        }
        for entry in walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("sql")
                && migrate_file_with_index(p, Some(&index))?
            {
                count += 1;
            }
        }
    }

    Ok(count)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_smelt_ref_to_path_form() {
        let input = "SELECT * FROM smelt.ref('users')";
        let expected = "SELECT * FROM smelt.models.users";
        assert_eq!(rewrite_content(input), expected);
    }

    #[test]
    fn rewrites_smelt_source_to_path_form() {
        let input = "FROM smelt.source('raw.events')";
        let expected = "FROM smelt.sources.raw.events";
        assert_eq!(rewrite_content(input), expected);
    }

    #[test]
    fn rewrites_smelt_fn_to_path_form() {
        let input = "SELECT smelt.fn.safe_divide(x, y) FROM t";
        let expected = "SELECT smelt.functions.safe_divide(x, y) FROM t";
        assert_eq!(rewrite_content(input), expected);
    }

    #[test]
    fn preserves_comment_lines() {
        let input = "-- smelt.ref('keep_this') unchanged\nSELECT * FROM smelt.ref('users')";
        let expected = "-- smelt.ref('keep_this') unchanged\nSELECT * FROM smelt.models.users";
        assert_eq!(rewrite_content(input), expected);
    }

    #[test]
    fn rewrites_double_quoted_ref() {
        let input = r#"SELECT * FROM smelt.ref("users")"#;
        let expected = "SELECT * FROM smelt.models.users";
        assert_eq!(rewrite_content(input), expected);
    }

    #[test]
    fn rewrites_nested_fn_path() {
        let input = "SELECT smelt.fn.core.safe_divide(a, b) FROM t";
        let expected = "SELECT smelt.functions.core.safe_divide(a, b) FROM t";
        assert_eq!(rewrite_content(input), expected);
    }

    #[test]
    fn no_change_when_already_migrated() {
        let input = "SELECT * FROM smelt.models.users";
        assert_eq!(rewrite_content(input), input);
    }

    #[test]
    fn model_index_resolves_subdirectory() {
        // Build a temporary workspace with a model in a subdirectory.
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("models").join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("stg_events.sql"), "SELECT 1").unwrap();

        let idx = ModelIndex::from_workspace(dir.path(), &["models"]).unwrap();
        assert_eq!(idx.resolve("stg_events"), "smelt.models.staging.stg_events");
    }

    #[test]
    fn model_index_resolves_multi_model_file() {
        let dir = tempfile::tempdir().unwrap();
        let models = dir.path().join("models");
        std::fs::create_dir_all(&models).unwrap();
        std::fs::write(
            models.join("pipeline.sql"),
            "--- name: staging_events ---\nSELECT 1\n--- name: cleaned_orders ---\nSELECT 2",
        )
        .unwrap();

        let idx = ModelIndex::from_workspace(dir.path(), &["models"]).unwrap();
        assert_eq!(idx.resolve("staging_events"), "smelt.models.staging_events");
        assert_eq!(idx.resolve("cleaned_orders"), "smelt.models.cleaned_orders");
    }

    #[test]
    fn rewrite_with_index_uses_full_path() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("models").join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("stg_events.sql"), "SELECT 1").unwrap();

        let idx = ModelIndex::from_workspace(dir.path(), &["models"]).unwrap();
        let input = "SELECT * FROM smelt.ref('stg_events')";
        let result = rewrite_content_with_index(input, Some(&idx));
        assert_eq!(result, "SELECT * FROM smelt.models.staging.stg_events");
    }
}
