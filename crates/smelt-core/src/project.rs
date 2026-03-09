use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("Could not find smelt project root.\nExpected to find 'smelt.yml' or 'models/' directory.\nHint: Run 'smelt init' to create a new project.")]
    ProjectRootNotFound,
}

/// Find a config file supporting both .yml and .yaml extensions.
/// Returns the path if exactly one exists. Errors if both exist.
pub fn find_config_file(dir: &Path, base_name: &str) -> Result<Option<PathBuf>, String> {
    let yml = dir.join(format!("{}.yml", base_name));
    let yaml = dir.join(format!("{}.yaml", base_name));
    match (yml.exists(), yaml.exists()) {
        (true, true) => Err(format!(
            "Both {}.yml and {}.yaml exist in {}",
            base_name,
            base_name,
            dir.display()
        )),
        (true, false) => Ok(Some(yml)),
        (false, true) => Ok(Some(yaml)),
        (false, false) => Ok(None),
    }
}

/// Find the smelt project root by walking up from start_dir.
/// Looks for smelt.yml, smelt.yaml, or a models/ directory.
pub fn find_project_root(start_dir: &Path) -> Result<PathBuf, ProjectError> {
    let mut current = start_dir.to_path_buf();

    // Walk up max 5 levels
    for _ in 0..5 {
        if current.join("smelt.yml").exists() || current.join("smelt.yaml").exists() {
            return Ok(current);
        }

        if current.join("models").is_dir() {
            return Ok(current);
        }

        if let Some(parent) = current.parent() {
            current = parent.to_path_buf();
        } else {
            break;
        }
    }

    Err(ProjectError::ProjectRootNotFound)
}

/// Walk up from a file path looking for smelt.yml/smelt.yaml to find project root.
/// Unlike `find_project_root`, this does not check for models/ directory and has no depth limit.
pub fn find_project_root_by_walking_up(file: &Path) -> Option<PathBuf> {
    let mut dir = file.parent()?;
    loop {
        if dir.join("smelt.yml").exists() || dir.join("smelt.yaml").exists() {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}

/// Recursively find directories containing smelt.yml/smelt.yaml
pub fn find_smelt_projects(root: &Path) -> Vec<PathBuf> {
    let mut projects = Vec::new();
    find_smelt_projects_recursive(root, &mut projects);
    projects
}

fn find_smelt_projects_recursive(dir: &Path, projects: &mut Vec<PathBuf>) {
    // Check if this directory has smelt.yml or smelt.yaml
    let has_yml = dir.join("smelt.yml").exists();
    let has_yaml = dir.join("smelt.yaml").exists();
    if has_yml || has_yaml {
        projects.push(dir.to_path_buf());
    }

    // Recurse into subdirectories, skipping common non-project dirs
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if matches!(name, ".git" | "target" | "node_modules") {
                        continue;
                    }
                }
                find_smelt_projects_recursive(&path, projects);
            }
        }
    }
}

/// Find the project root for a file by longest prefix match against known roots
pub fn find_project_root_for_file(file: &Path, roots: &[PathBuf]) -> Option<PathBuf> {
    roots
        .iter()
        .filter(|root| file.starts_with(root))
        .max_by_key(|root| root.components().count())
        .cloned()
}

/// Check if a file is a sources config file (sources.yml or sources.yaml)
pub fn is_sources_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n == "sources.yml" || n == "sources.yaml")
}
