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
