use std::fs;
use std::path::{Path, PathBuf};

use super::test_only_files;

const SCANNED_DIRS: &[&str] = &[
    "crates/smelt-logical/src/analysis",
    "crates/smelt-logical/src/maintenance",
];

fn collect_rs_files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(root, &path, out);
        } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
            out.push(
                path.strip_prefix(root)
                    .expect("scanned path is under repo root")
                    .to_path_buf(),
            );
        }
    }
}

pub(crate) fn scanned_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for dir in SCANNED_DIRS {
        collect_rs_files(root, &root.join(dir), &mut files);
    }
    files.retain(|rel| !test_only_files::is_test_only(root, rel));
    files.sort();
    files
}
