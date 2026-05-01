//! CLI entry point for the smelt path migration tool.
//!
//! Usage:
//!   migrate_smelt_path [dir1 dir2 ...]
//!
//! Each directory is treated as a workspace root.  The tool scans for a
//! `smelt.yml` to find model paths; if absent it defaults to `models/`.
//!
//! SQL files under those model paths (and `functions/`) are rewritten in-place.

use anyhow::Result;
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let dirs: Vec<PathBuf> = if args.is_empty() {
        vec![PathBuf::from("examples")]
    } else {
        args.iter().map(PathBuf::from).collect()
    };

    let mut total = 0;
    for dir in &dirs {
        if !dir.exists() {
            eprintln!("warning: directory {:?} does not exist, skipping", dir);
            continue;
        }
        // Read model paths from smelt.yml if present.
        let model_dirs = read_model_paths(dir);
        let model_dir_refs: Vec<&str> = model_dirs.iter().map(|s| s.as_str()).collect();
        let count = migrate_smelt_path::migrate_workspace(dir, &model_dir_refs)?;
        eprintln!("migrated {} file(s) in {}", count, dir.display());
        total += count;
    }

    eprintln!("total: {} file(s) modified", total);
    Ok(())
}

/// Read model_paths from `smelt.yml` in the workspace root.
/// Falls back to `["models"]` if the file is missing or unparseable.
fn read_model_paths(workspace_root: &std::path::Path) -> Vec<String> {
    let yml = workspace_root.join("smelt.yml");
    if !yml.exists() {
        return vec!["models".to_string()];
    }
    let content = match std::fs::read_to_string(&yml) {
        Ok(c) => c,
        Err(_) => return vec!["models".to_string()],
    };
    // Quick YAML parse: look for model_paths sequence.
    parse_model_paths_from_yml(&content).unwrap_or_else(|| vec!["models".to_string()])
}

fn parse_model_paths_from_yml(content: &str) -> Option<Vec<String>> {
    // Minimal YAML parse: find `model_paths:` section and extract list items.
    let start = content.find("model_paths:")?;
    let rest = &content[start + "model_paths:".len()..];
    let mut paths = Vec::new();
    for line in rest.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('-') {
            let val = trimmed.trim_start_matches('-').trim();
            if !val.is_empty() && !val.starts_with('#') {
                paths.push(val.to_string());
            }
        } else if !trimmed.starts_with('#') {
            // Non-list, non-comment line means end of sequence.
            break;
        }
    }
    if paths.is_empty() {
        None
    } else {
        Some(paths)
    }
}
