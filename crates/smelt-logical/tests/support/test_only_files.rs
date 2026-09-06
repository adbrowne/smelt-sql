//! Shared "is this file test-only?" classifier for structural gates that scan
//! `smelt-logical`'s admission/proof surface (`join_context_reach.rs`,
//! `walk_coverage.rs`) and for `.claude/scripts/hardening-budget.sh`'s awk
//! twin.
//!
//! The large-file splits on this branch turned `#[cfg(test)] mod tests { .. }`
//! *blocks* into whole *files* (e.g. `maintenance/choice/write_variant_tests.rs`,
//! declared as `#[cfg(test)] mod write_variant_tests;` in `choice/mod.rs`). No
//! gate's `#[cfg(test)]`-*span* scan (which only excludes inline blocks inside
//! a file) can see that — the whole file is test-only, so scanning its text as
//! production is wrong. This module derives test-only-ness from the
//! *declaration site* rather than a `*_tests.rs` naming convention, so a
//! differently-named split file is still classified correctly.
//!
//! The rule: a file at `<dir>/<stem>.rs` is test-only when the parent module
//! source (`<dir>/mod.rs`, else the sibling file `<parent-of-dir>/<dir-name>.rs`)
//! contains a `mod <stem>;` declaration whose own line, or the nearest
//! non-blank line above it, is `#[cfg(test)]`. This is applied transitively up
//! the directory chain: if `<dir>` itself is declared under `#[cfg(test)]` in
//! *its* parent, every file under `<dir>` is test-only too, regardless of how
//! that file's own `mod` declaration reads.
//!
//! Fails loud, never skips: if no parent module declaration can be found at
//! all (unreadable or absent parent module file), the file is classified as
//! production. Silently excluding an undeclared file would reopen exactly the
//! blind spot this module exists to close.

use std::fs;
use std::path::Path;

/// The parent module source text for `dir` — `<dir>/mod.rs` if present, else
/// the sibling file `<dir's parent>/<dir's own name>.rs`. `None` if neither
/// exists or is readable.
fn parent_module_source(dir: &Path) -> Option<String> {
    let mod_rs = dir.join("mod.rs");
    if let Ok(text) = fs::read_to_string(&mod_rs) {
        return Some(text);
    }
    let name = dir.file_name()?.to_string_lossy().to_string();
    let sibling = dir.parent()?.join(format!("{name}.rs"));
    fs::read_to_string(&sibling).ok()
}

/// Does `parent_src` declare `mod <stem>;` (optionally `pub mod <stem>;`)
/// under `#[cfg(test)]` — either on the same line (`#[cfg(test)] mod
/// <stem>;`) or on the nearest non-blank line above the declaration?
pub fn declared_cfg_test(parent_src: &str, stem: &str) -> bool {
    let lines: Vec<&str> = parent_src.lines().collect();
    let needle = format!("mod {stem};");
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if !trimmed.contains(&needle) {
            continue;
        }
        if trimmed.starts_with("#[cfg(test)]") {
            return true;
        }
        let mut j = i;
        while j > 0 {
            j -= 1;
            let prev = lines[j].trim();
            if prev.is_empty() {
                continue;
            }
            return prev == "#[cfg(test)]" || prev.starts_with("#[cfg(test)]");
        }
        return false;
    }
    false
}

/// Is the file at `repo_root.join(rel_path)` test-only, per the rule above,
/// applied transitively up the directory chain?
pub fn is_test_only(repo_root: &Path, rel_path: &Path) -> bool {
    let abs = repo_root.join(rel_path);
    let Some(dir) = abs.parent() else {
        return false;
    };
    let Some(stem) = abs.file_stem().map(|s| s.to_string_lossy().to_string()) else {
        return false;
    };
    if let Some(src) = parent_module_source(dir) {
        if declared_cfg_test(&src, &stem) {
            return true;
        }
    }

    // Transitivity: walk up the directory chain. Each step asks whether the
    // current directory's own name is declared under #[cfg(test)] in *its*
    // parent's module source. Stops the moment no further module source can
    // be found (we've walked out of the crate's module tree).
    let mut current = dir.to_path_buf();
    loop {
        let Some(name) = current.file_name().map(|n| n.to_string_lossy().to_string()) else {
            return false;
        };
        let Some(parent) = current.parent() else {
            return false;
        };
        match parent_module_source(parent) {
            None => return false,
            Some(src) => {
                if declared_cfg_test(&src, &name) {
                    return true;
                }
            }
        }
        current = parent.to_path_buf();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_under_cfg_test_is_test_only() {
        let parent_src = "mod mod_a;\n#[cfg(test)]\nmod tests;\nmod mod_b;\n";
        assert!(declared_cfg_test(parent_src, "tests"));
    }

    #[test]
    fn plain_mod_declaration_is_production() {
        let parent_src = "mod mod_a;\nmod real;\nmod mod_b;\n";
        assert!(!declared_cfg_test(parent_src, "real"));
    }

    #[test]
    fn same_line_cfg_test_is_test_only() {
        let parent_src = "mod mod_a;\n#[cfg(test)] mod tests;\nmod mod_b;\n";
        assert!(declared_cfg_test(parent_src, "tests"));
    }

    #[test]
    fn nested_under_test_only_module_is_test_only() {
        let dir = std::env::temp_dir().join(format!(
            "smelt-test-only-files-nested-{}",
            std::process::id()
        ));
        let m_dir = dir.join("src/m");
        let helper_dir = m_dir.join("helper_dir");
        fs::create_dir_all(&helper_dir).expect("create nested dirs");
        fs::write(
            m_dir.join("mod.rs"),
            "mod real;\n#[cfg(test)]\nmod helper_dir;\n",
        )
        .expect("write m/mod.rs");
        fs::write(helper_dir.join("mod.rs"), "mod inner;\n").expect("write helper_dir/mod.rs");
        fs::write(helper_dir.join("inner.rs"), "pub fn f() {}\n").expect("write inner.rs");

        let inner_rel = Path::new("src/m/helper_dir/inner.rs");
        assert!(is_test_only(&dir, inner_rel));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn undeclared_file_is_production() {
        let dir = std::env::temp_dir().join(format!(
            "smelt-test-only-files-undeclared-{}",
            std::process::id()
        ));
        let src_dir = dir.join("src");
        fs::create_dir_all(&src_dir).expect("create src dir");
        fs::write(src_dir.join("orphan.rs"), "pub fn f() {}\n").expect("write orphan.rs");

        let rel = Path::new("src/orphan.rs");
        assert!(!is_test_only(&dir, rel));

        let _ = fs::remove_dir_all(&dir);
    }
}
