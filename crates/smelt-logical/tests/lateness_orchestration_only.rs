//! Structural gate for `docs/specs/model_properties.md` §Constraints
//! "Declared lateness is orchestration-only": no plan-derivation code in
//! `smelt-logical` reads a source's declared lateness. Declared lateness
//! (`mutation_profile.lateness`/`source_lateness` on a source,
//! `ColumnMetadata::data_latency` — retired — on a model column) is an
//! orchestration-only fact; it never widens a derived bound or feeds a
//! composition-relevant verdict.
//!
//! Mechanism: scan every production line (`#[cfg(test)]` module bodies
//! excluded, mirroring `walk_coverage.rs`'s convention) under
//! `crates/smelt-logical/src` for a `.lateness`/`.source_lateness`/
//! `.data_latency` *field read*. A `some_struct { source_lateness: None, .. }`
//! fixture literal (no leading dot — it's a field *name* in a struct
//! constructor, not a read off a value) and a doc-comment line are excluded.

use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/smelt-logical has a parent dir")
        .parent()
        .expect("crates/ has a parent dir")
        .to_path_buf()
}

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

/// Production text: everything before the file's `#[cfg(test)]` module (if
/// any), mirroring `walk_coverage.rs`'s convention for excluding test bodies.
fn production_text(source: &str) -> &str {
    match source.find("#[cfg(test)]") {
        Some(idx) => &source[..idx],
        None => source,
    }
}

const FIELD_PATTERNS: &[&str] = &[".lateness", ".source_lateness", ".data_latency"];

#[test]
fn no_plan_derivation_reads_lateness() {
    let root = repo_root();
    let mut files = Vec::new();
    collect_rs_files(&root, &root.join("crates/smelt-logical/src"), &mut files);
    files.sort();

    let mut offenders = Vec::new();
    for rel in &files {
        let full = root.join(rel);
        let text = fs::read_to_string(&full).unwrap_or_default();
        let production = production_text(&text);
        for (i, line) in production.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue; // doc/line comment
            }
            for pat in FIELD_PATTERNS {
                if line.contains(pat) {
                    offenders.push(format!("{}:{}: {}", rel.display(), i + 1, line.trim()));
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "no plan-derivation code in smelt-logical may read declared lateness \
         (`docs/specs/model_properties.md` §Constraints \"Declared lateness is \
         orchestration-only\"), found:\n  {}",
        offenders.join("\n  ")
    );
}

/// `compute_effective_window`'s own signature carries no latency parameter —
/// pinned directly rather than only via the field-read scan above, since a
/// caller could in principle thread a latency value through without ever
/// spelling `.lateness` at the call site.
#[test]
fn compute_effective_window_signature_has_no_latency_parameter() {
    let root = repo_root();
    let text =
        fs::read_to_string(root.join("crates/smelt-logical/src/analysis/temporal.rs")).unwrap();
    let sig_start = text
        .find("pub fn compute_effective_window(")
        .expect("compute_effective_window must exist");
    let sig_end = text[sig_start..]
        .find(") -> EffectiveWindow")
        .expect("compute_effective_window must return EffectiveWindow");
    let signature = &text[sig_start..sig_start + sig_end];
    assert!(
        !signature.to_lowercase().contains("latency"),
        "compute_effective_window's signature must carry no latency input, got: {signature}"
    );
}
