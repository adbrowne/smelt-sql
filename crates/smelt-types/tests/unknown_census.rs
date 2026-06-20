/// Guard: every DataType::Unknown CONSTRUCTION site in production code must be
/// classified in `.claude/unknown-census.toml` as either `legitimate` or `error`.
///
/// A "construction site" is a line that *produces* `DataType::Unknown` — struct
/// field initialiser, `unwrap_or` fallback, return value, match arm body, etc.
/// Pattern-match arms (`DataType::Unknown(_) =>`), comparison guards (`matches!`,
/// `== DataType::unknown_dynamic()`), and test code are excluded.
///
/// An unclassified new site fails this test (prevent silent-Unknown growth).
/// A missing allowlist entry (debt was paid) also fails (force tightening).
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn census_script() -> PathBuf {
    repo_root().join(".claude/scripts/unknown-census.sh")
}

fn allowlist_path() -> PathBuf {
    repo_root().join(".claude/unknown-census.toml")
}

/// Parse a simple TOML-like allowlist.  Format per entry:
///
/// ```toml
/// ["crates/smelt-foo/src/bar.rs:42"]
/// classification = "legitimate"  # or "error"
/// reason = "one-line explanation"
/// ```
struct CensusEntry {
    classification: String,
    note: String,
    discriminant: String,
}

fn load_allowlist(path: &std::path::Path) -> HashMap<String, CensusEntry> {
    let text = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("Cannot read allowlist at {path:?}: {e}"));

    let mut entries: HashMap<String, CensusEntry> = HashMap::new();
    let mut current_key: Option<String> = None;
    let mut current_class = String::new();
    let mut current_reason = String::new();
    let mut current_disc = String::new();

    for raw_line in text.lines() {
        let line = raw_line.trim();

        if line.starts_with('#') || line.is_empty() {
            continue;
        }

        // Section header: ["crates/..."]
        if line.starts_with("[\"") && line.ends_with("\"]") {
            if let Some(key) = current_key.take() {
                entries.insert(
                    key,
                    CensusEntry {
                        classification: current_class.clone(),
                        note: current_reason.clone(),
                        discriminant: current_disc.clone(),
                    },
                );
            }
            current_key = Some(line[2..line.len() - 2].to_string());
            current_class.clear();
            current_reason.clear();
            current_disc.clear();
            continue;
        }

        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim();
            let v = v.trim().trim_matches('"');
            match k {
                "classification" => current_class = v.to_string(),
                "reason" => current_reason = v.to_string(),
                "discriminant" => current_disc = v.to_string(),
                _ => {}
            }
        }
    }

    if let Some(key) = current_key {
        entries.insert(
            key,
            CensusEntry {
                classification: current_class,
                note: current_reason,
                discriminant: current_disc,
            },
        );
    }

    entries
}

/// Run the census script and collect the set of site strings it outputs.
fn run_census() -> HashSet<String> {
    let root = repo_root();
    let script = census_script();

    assert!(
        script.exists(),
        "unknown-census.sh not found at {script:?}\n\
         Run: bash .claude/scripts/unknown-census.sh to confirm it exists (Phase 2 pre-req)"
    );

    let out = Command::new("bash")
        .arg(&script)
        .env("REPO_ROOT", &root)
        .current_dir(&root)
        .output()
        .expect("failed to execute unknown-census.sh");

    assert!(
        out.status.success(),
        "unknown-census.sh exited non-zero:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

#[test]
fn every_unknown_site_is_classified() {
    let allowlist_file = allowlist_path();
    assert!(
        allowlist_file.exists(),
        "unknown-census.toml not found at {allowlist_file:?}\n\
         Create it by running the census script and classifying each site."
    );

    let allowlist = load_allowlist(&allowlist_file);
    let found = run_census();

    let mut failures: Vec<String> = Vec::new();

    // Every found site must be in the allowlist
    let mut unregistered: Vec<&str> = found
        .iter()
        .filter(|s| !allowlist.contains_key(s.as_str()))
        .map(|s| s.as_str())
        .collect();
    unregistered.sort_unstable();
    for site in &unregistered {
        failures.push(format!(
            "UNREGISTERED: {site}\n  \
             Add it to .claude/unknown-census.toml with classification + reason."
        ));
    }

    // Every allowlist entry must still exist in the found set (two-sided ratchet)
    let mut stale: Vec<&str> = allowlist
        .keys()
        .filter(|k| !found.contains(k.as_str()))
        .map(|k| k.as_str())
        .collect();
    stale.sort_unstable();
    for site in &stale {
        let entry = allowlist.get(*site).unwrap();
        failures.push(format!(
            "STALE ALLOWLIST: {site} (was: {}, \"{}\")\n  \
             Remove it or update the line number in .claude/unknown-census.toml.",
            entry.classification, entry.note
        ));
    }

    // Every entry must have a non-empty classification
    let mut bad_class: Vec<&str> = allowlist
        .iter()
        .filter(|(_, e)| e.classification != "legitimate" && e.classification != "error")
        .map(|(k, _)| k.as_str())
        .collect();
    bad_class.sort_unstable();
    for site in &bad_class {
        let entry = allowlist.get(*site).unwrap();
        failures.push(format!(
            "INVALID CLASSIFICATION: {site} has \"{}\" — must be \"legitimate\" or \"error\"",
            entry.classification
        ));
    }

    assert!(
        failures.is_empty(),
        "unknown-census failures:\n{}\n\n\
         Run `.claude/scripts/unknown-census.sh` to see the current production sites.\n\
         Edit `.claude/unknown-census.toml` to bring the allowlist in sync.",
        failures.join("\n")
    );
}

/// Every census entry must declare a `discriminant` — one of the closed
/// `UnknownReason` values: `unresolved`, `dynamic`, or `propagated`.
///
/// A missing or invalid discriminant is an error: add
/// `discriminant = "unresolved" | "dynamic" | "propagated"` to the entry.
#[test]
fn every_site_declares_a_discriminant() {
    let allowlist = load_allowlist(&allowlist_path());
    let valid = ["unresolved", "dynamic", "propagated"];
    let mut bad: Vec<String> = allowlist
        .iter()
        .filter(|(_, e)| !valid.contains(&e.discriminant.as_str()))
        .map(|(site, e)| {
            format!(
                "MISSING/INVALID DISCRIMINANT: {site} has {:?} — must be one of: unresolved, dynamic, propagated",
                e.discriminant
            )
        })
        .collect();
    bad.sort();
    assert!(
        bad.is_empty(),
        "Sites without a valid discriminant (add `discriminant = \"...\"` to each):\n{}",
        bad.join("\n")
    );
}

/// Any `classification = "error"` site must also declare
/// `discriminant = "unresolved"` — an error site is a compiler-resolvable gap
/// whose `Unknown` is not yet converted to a `ColumnTypeUnresolved` diagnostic.
#[test]
fn error_classification_implies_unresolved_discriminant() {
    let allowlist = load_allowlist(&allowlist_path());
    let mut bad: Vec<String> = allowlist
        .iter()
        .filter(|(_, e)| e.classification == "error" && e.discriminant != "unresolved")
        .map(|(site, e)| {
            format!(
                "ERROR SITE NOT UNRESOLVED: {site} (classification=error but discriminant={:?})",
                e.discriminant
            )
        })
        .collect();
    bad.sort();
    assert!(
        bad.is_empty(),
        "error-classified sites must have discriminant=unresolved:\n{}",
        bad.join("\n")
    );
}
