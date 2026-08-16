/// Coverage gate: every `DiagnosticCode` variant must appear in the
/// `docs/specs/diagnostics.md` catalogue wrapped in backticks.
///
/// The test mirrors the `hardening_budget` gate style: scan the enum source
/// with plain string / char operations (no external regex crate), collect
/// variant names, then assert each one appears as `` `VariantName` `` in the
/// catalogue.
#[test]
fn every_diagnostic_code_is_catalogued() {
    // ── 1. Parse variant names from the enum source ──────────────────────
    let source_path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/diagnostics_types.rs");
    let source =
        std::fs::read_to_string(source_path).expect("should be able to read diagnostics_types.rs");

    let mut variants: Vec<String> = Vec::new();
    let mut inside_enum = false;
    let mut brace_depth: usize = 0;

    for line in source.lines() {
        let trimmed = line.trim();

        if !inside_enum {
            // Detect the opening line of the enum.
            if trimmed.starts_with("pub enum DiagnosticCode {") {
                inside_enum = true;
                brace_depth = 1; // the opening `{` on this same line
            }
            continue;
        }

        // Skip doc-comment / line-comment / attribute lines entirely — both for
        // variant extraction AND for brace counting. Doc comments can contain
        // type examples like `Struct<{a: Integer}>`; counting their braces would
        // corrupt `brace_depth` and could break the enum scan early, silently
        // dropping later variants from the enforced set (a false GREEN for this
        // rot-prevention gate).
        if trimmed.is_empty()
            || trimmed.starts_with("///")
            || trimmed.starts_with("//")
            || trimmed.starts_with("#[")
        {
            continue;
        }

        // Track brace depth (on real code lines only) to find the enum's
        // closing `}`.
        for ch in trimmed.chars() {
            if ch == '{' {
                brace_depth += 1;
            } else if ch == '}' {
                brace_depth -= 1;
            }
        }

        // When we return to depth 0 we've passed the enum's closing brace.
        if brace_depth == 0 {
            break;
        }

        // Skip any remaining lines that contain `{` or `}` (struct variants or
        // inner blocks) — they are not bare unit variants.
        if trimmed.contains('{') || trimmed.contains('}') {
            continue;
        }

        // A unit variant line looks like `VariantName,` after trimming.
        if let Some(name) = trimmed.strip_suffix(',') {
            // Must be a valid PascalCase identifier: starts uppercase,
            // all chars alphanumeric or underscore, not empty.
            if name.is_empty() {
                continue;
            }
            let first = name.chars().next().unwrap();
            if first.is_uppercase() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                variants.push(name.to_string());
            }
        }
    }

    // ── 2. Sanity-assert the parser found a plausible number of variants ──
    assert!(
        variants.len() >= 140,
        "variant parser looks broken — only found {} variants (expected >= 140)",
        variants.len()
    );

    // ── 3. Read the catalogue ─────────────────────────────────────────────
    let catalogue_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/specs/diagnostics.md"
    );
    let catalogue = std::fs::read_to_string(catalogue_path)
        .expect("should be able to read docs/specs/diagnostics.md");

    // ── 4. Assert every variant appears as `VariantName` in the catalogue ─
    let missing: Vec<String> = variants
        .iter()
        .filter(|v| !catalogue.contains(&format!("`{}`", v)))
        .cloned()
        .collect();

    if !missing.is_empty() {
        let mut sorted = missing.clone();
        sorted.sort();
        panic!(
            "{} DiagnosticCode variant(s) are not documented in docs/specs/diagnostics.md \
             (each must appear as `VariantName` in a backtick-wrapped token):\n  {}",
            sorted.len(),
            sorted.join("\n  ")
        );
    }
}

/// Standing ratchet (mirrors `rebuild_dry_run.rs`'s `no_backbuild_verb_in_user_docs`):
/// the retired skeleton-change diagnostic-code name must never reappear in
/// `crates/`, `docs/specs/`, or `docs-site/docs/`. `docs/plans/` and
/// `docs/outcomes/` are historical and excluded. The retired name is built
/// from parts so this test's own source does not trip the scan it performs.
#[test]
fn no_old_skeleton_code_name_in_specs_or_code() {
    let repo_root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let repo_root = std::path::Path::new(repo_root)
        .canonicalize()
        .expect("repo root exists");

    let old_name = ["MaintenanceSkeleton", "ColumnAdded"].concat();
    let mut offenders = Vec::new();
    for dir in ["crates", "docs/specs", "docs-site/docs"] {
        let root = repo_root.join(dir);
        for entry in walkdir(&root) {
            if entry.ends_with("tests/integration/diagnostics_catalogue.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&entry).unwrap_or_default();
            for (i, line) in text.lines().enumerate() {
                if line.contains(&old_name) {
                    offenders.push(format!("{}:{}: {}", entry.display(), i + 1, line));
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "found retired diagnostic-code name `{old_name}` still referenced:\n{}",
        offenders.join("\n")
    );
}

fn walkdir(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if !root.exists() {
        return out;
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n == "target" || n == "node_modules" || n == "site")
                {
                    continue;
                }
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out
}
