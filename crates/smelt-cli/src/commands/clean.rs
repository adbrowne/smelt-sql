//! `smelt clean` — remove build artifacts under `target/`.
//!
//! Never touches `.smelt/` (run manifests, deployed-schema snapshots) or the
//! configured target database (`docs/specs/cli.md` §"`smelt clean`",
//! Constraints & Invariants item 17). The deletion set is the single,
//! enumerated `target/` directory smelt itself writes to (`smelt docs
//! generate` and other artifact-producing commands) — never a glob outside
//! it.

use anyhow::{Context, Result};
use smelt_cli::find_project_root;

use crate::CleanArgs;

pub async fn clean(args: CleanArgs) -> Result<()> {
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    let target_dir = project_dir.join("target");
    if !target_dir.exists() {
        println!("Nothing to clean (no target/ directory).");
        return Ok(());
    }

    let mut removed: Vec<String> = Vec::new();
    collect_paths(&target_dir, &target_dir, &mut removed)?;
    removed.sort();

    std::fs::remove_dir_all(&target_dir)
        .with_context(|| format!("Failed to remove {}", target_dir.display()))?;

    println!("Removed target/:");
    for path in &removed {
        println!("  {path}");
    }

    Ok(())
}

/// Recursively collect file paths under `dir` (relative to `root`) for the
/// "what was deleted" report, without deleting anything.
fn collect_paths(
    root: &std::path::Path,
    dir: &std::path::Path,
    out: &mut Vec<String>,
) -> Result<()> {
    for entry in
        std::fs::read_dir(dir).with_context(|| format!("Failed to read {}", dir.display()))?
    {
        let entry = entry.with_context(|| format!("Failed to read entry in {}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_paths(root, &path, out)?;
        } else {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            out.push(rel.display().to_string());
        }
    }
    Ok(())
}
