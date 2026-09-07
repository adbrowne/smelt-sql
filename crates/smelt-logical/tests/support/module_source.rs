//! Resolves a module's production source text whether the module is still a
//! single file or has been split into a directory — the shape gates like
//! `contract_lattice_spec.rs` need after a large-file split turns
//! `<stem>.rs` into `<stem>/mod.rs` + siblings and a gate that reads the old
//! single path goes stale (path drift, not a real regression).
//!
//! `read_module(repo_root, rel_stem)` resolves `<rel_stem>.rs` if that file
//! still exists; otherwise it concatenates every non-test-only `.rs` file
//! under the `<rel_stem>/` directory (test-only-ness per
//! `test_only_files::is_test_only`, so a symbol that only exists in a split
//! `#[cfg(test)] mod tests;` file cannot satisfy a production-surface
//! assertion). Panics loudly, naming the stem, if neither form exists —
//! silently returning empty text would turn every assertion against it into
//! a false negative.

use std::fs;
use std::path::Path;

use super::test_only_files::is_test_only;

fn rs_files_recursive(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("read {dir:?}: {e}")) {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            out.extend(rs_files_recursive(&path));
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
    out
}

/// Read the production source of the module named `rel_stem` (a repo-relative
/// path with no `.rs` extension, e.g. `crates/smelt-logical/src/contract/frozen_horizon`).
pub fn read_module(repo_root: &Path, rel_stem: &str) -> String {
    let file_path = repo_root.join(format!("{rel_stem}.rs"));
    if let Ok(text) = fs::read_to_string(&file_path) {
        return text;
    }

    let dir_path = repo_root.join(rel_stem);
    if dir_path.is_dir() {
        let mut combined = String::new();
        for path in rs_files_recursive(&dir_path) {
            let rel_path = path
                .strip_prefix(repo_root)
                .unwrap_or_else(|e| panic!("{path:?} is not under {repo_root:?}: {e}"));
            if is_test_only(repo_root, rel_path) {
                continue;
            }
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
            combined.push_str(&text);
            combined.push('\n');
        }
        return combined;
    }

    panic!(
        "module {rel_stem:?} not found as either {} or a directory at {}",
        file_path.display(),
        dir_path.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("smelt-module-source-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn read_module_resolves_a_single_file_module() {
        let dir = temp_dir("single-file");
        let src_dir = dir.join("src");
        fs::create_dir_all(&src_dir).expect("create src dir");
        fs::write(src_dir.join("widget.rs"), "pub fn widget_fn() {}\n").expect("write widget.rs");

        let text = read_module(&dir, "src/widget");
        assert!(text.contains("pub fn widget_fn"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_module_resolves_a_split_directory_module() {
        let dir = temp_dir("split-dir");
        let widget_dir = dir.join("src/widget");
        fs::create_dir_all(&widget_dir).expect("create widget dir");
        fs::write(
            widget_dir.join("mod.rs"),
            "mod sibling;\npub fn mod_fn() {}\n",
        )
        .expect("write mod.rs");
        fs::write(widget_dir.join("sibling.rs"), "pub fn sibling_fn() {}\n")
            .expect("write sibling.rs");

        let text = read_module(&dir, "src/widget");
        assert!(text.contains("pub fn mod_fn"));
        assert!(text.contains("pub fn sibling_fn"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_module_excludes_test_only_files() {
        let dir = temp_dir("excludes-test-only");
        let widget_dir = dir.join("src/widget");
        fs::create_dir_all(&widget_dir).expect("create widget dir");
        fs::write(
            widget_dir.join("mod.rs"),
            "pub fn mod_fn() {}\n#[cfg(test)]\nmod tests;\n",
        )
        .expect("write mod.rs");
        fs::write(
            widget_dir.join("tests.rs"),
            "pub fn only_in_tests_fn() {}\n",
        )
        .expect("write tests.rs");

        let text = read_module(&dir, "src/widget");
        assert!(text.contains("pub fn mod_fn"));
        assert!(!text.contains("only_in_tests_fn"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[should_panic(expected = "src/widget/missing")]
    fn read_module_panics_when_the_module_is_absent() {
        let dir = temp_dir("absent");
        fs::create_dir_all(&dir).expect("create dir");
        let _ = read_module(&dir, "src/widget/missing");
    }
}
