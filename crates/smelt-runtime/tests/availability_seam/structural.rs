//! Structural: no production `smelt-runtime` module outside
//! `maintenance_availability.rs` itself calls `smelt-db`'s raw
//! `derive_model_maintenance_plan{,_with_edges}` — every consumer goes
//! through the seam.

#[test]
fn every_runtime_derivation_goes_through_the_availability_seam() {
    let src_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let needle = "smelt_db::queries::maintenance::derive_model_maintenance_plan";
    let mut offenders = Vec::new();
    for entry in walk_rs_files(&src_dir) {
        // The seam itself: `maintenance_availability.rs`, now split into
        // `maintenance_availability/{mod,derive_resolved,
        // derive_resolved_with_edges}.rs` — every file under that directory
        // is the seam, not an offender, so exclude by parent directory name
        // rather than by a single filename that only matched before the
        // split.
        if entry
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            == Some("maintenance_availability")
        {
            continue;
        }
        let text = std::fs::read_to_string(&entry).expect("read source file");
        for (line_no, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("///") || trimmed.starts_with("//!") || trimmed.starts_with("//")
            {
                continue;
            }
            if line.contains(needle) {
                offenders.push(format!("{}:{}", entry.display(), line_no + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "found a call to the raw smelt-db maintenance-plan derivation outside the availability \
         seam: {offenders:?}"
    );
}

/// Phase 6 (`docs/outcomes/20260904-state-residency/outcome.md`): the
/// recorded `state_downgrade` is now genuinely user-visible via `smelt
/// explain`, so the retired reporter stand-in method (the one this test
/// searches for by name, built from parts below so this file itself does
/// not reintroduce a hit) is gone entirely — not just unused. Structural:
/// zero occurrences (including comments) in `smelt-runtime` or `smelt-cli`
/// production/test source, this file excluded (it necessarily names the
/// retired method once, in the assembled `needle`).
#[test]
fn retired_reporter_stub_leaves_no_trace() {
    // Assembled at runtime, not a contiguous literal, so this file itself
    // is not a false-positive hit for its own search.
    let needle = ["state_structure_un", "available"].concat();
    let mut offenders = Vec::new();
    for crate_dir in ["src", "tests"] {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(crate_dir);
        for entry in walk_rs_files(&dir) {
            if entry.file_name().and_then(|n| n.to_str()) == Some("structural.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&entry).expect("read source file");
            for (line_no, line) in text.lines().enumerate() {
                if line.contains(&needle) {
                    offenders.push(format!("{}:{}", entry.display(), line_no + 1));
                }
            }
        }
    }
    let cli_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../smelt-cli/src");
    for entry in walk_rs_files(&cli_src) {
        let text = std::fs::read_to_string(&entry).expect("read source file");
        for (line_no, line) in text.lines().enumerate() {
            if line.contains(&needle) {
                offenders.push(format!("{}:{}", entry.display(), line_no + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "found a remaining occurrence of the retired reporter method: {offenders:?}"
    );
}

fn walk_rs_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk_rs_files(&path));
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
    out
}
