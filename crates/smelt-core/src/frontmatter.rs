//! Unified frontmatter parser over a key catalogue.
//!
//! `parse_frontmatter(text, kind)` is the single entry point for all
//! frontmatter parsing in smelt. Callers pass the raw YAML text and the
//! kind of declaration being parsed; the function validates every key against
//! a static catalogue and returns a filtered mapping plus diagnostics.
//!
//! # Policy (§"Unified frontmatter rule" in docs/specs/architecture.md)
//! - Unknown key (not in the catalogue at all) → `FrontmatterSeverity::Error`
//! - Known key inapplicable to the declaration kind → `FrontmatterSeverity::Warning`
//! - Known key applicable to the declaration kind → kept in the validated map
//! - Malformed YAML or non-mapping top level → `FrontmatterSeverity::Error`
//! - Nothing is silently dropped

use serde_yaml;

/// Which kind of smelt declaration is being parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclarationKind {
    /// A `.sql` model file.
    Model,
    /// A `smelt.define` function declaration.
    Define,
    /// A `smelt.extern` function reference.
    Extern,
}

/// A diagnostic produced by the frontmatter parser.
///
/// Kept minimal (no spans) so the type can live in `smelt-core` independent
/// of Salsa or the LSP. The Salsa layer in `smelt-db` anchors these at the
/// declaring node's name range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontmatterDiagnostic {
    /// Severity — `Error` for fatal failures, `Warning` for inapplicable keys.
    pub severity: FrontmatterSeverity,
    /// Human-readable message passed through to the final `Diagnostic.message`.
    pub message: String,
}

/// Severity level for a [`FrontmatterDiagnostic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontmatterSeverity {
    /// The frontmatter could not be parsed at all, or a key is unknown.
    Error,
    /// A key is known but not applicable to this declaration kind; the rest
    /// of the block parses normally.
    Warning,
}

// ── Key catalogue ─────────────────────────────────────────────────────────────

// Each entry is (key, applicable_kinds). A key absent from this table is
// "unknown" and produces an Error. A key present but not applicable to the
// caller's DeclarationKind produces a Warning and is excluded from the
// validated map.
static CATALOGUE: &[(&str, &[DeclarationKind])] = &[
    // Model-only keys (match the fields of ModelMetadata).
    ("name", &[DeclarationKind::Model]),
    ("generates", &[DeclarationKind::Model]),
    ("materialization", &[DeclarationKind::Model]),
    ("timeseries", &[DeclarationKind::Model]),
    ("incremental", &[DeclarationKind::Model]),
    ("target", &[DeclarationKind::Model]),
    ("owner", &[DeclarationKind::Model]),
    ("description", &[DeclarationKind::Model]),
    ("columns", &[DeclarationKind::Model]),
    ("backend_hints", &[DeclarationKind::Model]),
    ("test", &[DeclarationKind::Model]),
    ("schema_evolution", &[DeclarationKind::Model]),
    ("format", &[DeclarationKind::Model]),
    // Function/extern-only keys (match the fields of FunctionProperties).
    (
        "deterministic",
        &[DeclarationKind::Define, DeclarationKind::Extern],
    ),
    (
        "idempotent",
        &[DeclarationKind::Define, DeclarationKind::Extern],
    ),
    (
        "append_only",
        &[DeclarationKind::Define, DeclarationKind::Extern],
    ),
    (
        "needs_cast",
        &[DeclarationKind::Define, DeclarationKind::Extern],
    ),
    (
        "provenance",
        &[DeclarationKind::Define, DeclarationKind::Extern],
    ),
    ("joins", &[DeclarationKind::Define, DeclarationKind::Extern]),
    (
        "backends",
        &[DeclarationKind::Define, DeclarationKind::Extern],
    ),
    // Shared keys — applicable to all declaration kinds.
    (
        "tags",
        &[
            DeclarationKind::Model,
            DeclarationKind::Define,
            DeclarationKind::Extern,
        ],
    ),
];

fn catalogue_lookup(key: &str) -> Option<&'static [DeclarationKind]> {
    CATALOGUE
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, kinds)| *kinds)
}

fn yaml_kind_name(v: &serde_yaml::Value) -> &'static str {
    match v {
        serde_yaml::Value::Null => "null",
        serde_yaml::Value::Bool(_) => "boolean",
        serde_yaml::Value::Number(_) => "number",
        serde_yaml::Value::String(_) => "string",
        serde_yaml::Value::Sequence(_) => "sequence",
        serde_yaml::Value::Mapping(_) => "mapping",
        serde_yaml::Value::Tagged(_) => "tagged",
    }
}

/// Parse the frontmatter YAML block `text` for a declaration of `kind`.
///
/// Returns a `(validated_map, diagnostics)` pair:
/// - `validated_map` contains only the keys that are in the catalogue **and**
///   applicable to `kind`.
/// - `diagnostics` carries one entry per unknown key (Error) and one per
///   inapplicable key (Warning). On a YAML-level parse failure the returned
///   map is empty and a single Error carries the underlying message.
///
/// **No caller is wired to this function yet** (U2). Callers are connected in
/// phases U3 and U4.
pub fn parse_frontmatter(
    text: &str,
    kind: DeclarationKind,
) -> (serde_yaml::Mapping, Vec<FrontmatterDiagnostic>) {
    let mut diags: Vec<FrontmatterDiagnostic> = Vec::new();
    let mut validated = serde_yaml::Mapping::new();

    if text.trim().is_empty() {
        return (validated, diags);
    }

    let value: serde_yaml::Value = match serde_yaml::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            diags.push(FrontmatterDiagnostic {
                severity: FrontmatterSeverity::Error,
                message: format!("frontmatter: failed to parse YAML: {e}"),
            });
            return (validated, diags);
        }
    };

    let mapping = match value {
        serde_yaml::Value::Mapping(m) => m,
        serde_yaml::Value::Null => return (validated, diags),
        other => {
            diags.push(FrontmatterDiagnostic {
                severity: FrontmatterSeverity::Error,
                message: format!(
                    "frontmatter: expected a top-level mapping, got {}",
                    yaml_kind_name(&other)
                ),
            });
            return (validated, diags);
        }
    };

    let kind_name = match kind {
        DeclarationKind::Model => "model",
        DeclarationKind::Define => "define",
        DeclarationKind::Extern => "extern",
    };

    for (k, v) in mapping {
        let key = match k.as_str() {
            Some(s) => s.to_string(),
            None => {
                diags.push(FrontmatterDiagnostic {
                    severity: FrontmatterSeverity::Error,
                    message: "frontmatter: non-string key in top-level mapping".to_string(),
                });
                continue;
            }
        };

        match catalogue_lookup(&key) {
            None => {
                diags.push(FrontmatterDiagnostic {
                    severity: FrontmatterSeverity::Error,
                    message: format!("frontmatter: unknown key '{key}'"),
                });
                // Key is not added to validated map.
            }
            Some(applicable) => {
                if applicable.contains(&kind) {
                    validated.insert(serde_yaml::Value::String(key), v);
                } else {
                    diags.push(FrontmatterDiagnostic {
                        severity: FrontmatterSeverity::Warning,
                        message: format!(
                            "frontmatter: key '{key}' is not applicable to {kind_name} declarations"
                        ),
                    });
                    // Key is not added to validated map.
                }
            }
        }
    }

    (validated, diags)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_key(map: &serde_yaml::Mapping, key: &str) -> bool {
        map.contains_key(serde_yaml::Value::String(key.to_string()))
    }

    #[test]
    fn unknown_key_produces_error() {
        let (map, diags) = parse_frontmatter("typo_key: true", DeclarationKind::Model);
        assert_eq!(diags.len(), 1, "expected exactly one diagnostic");
        assert_eq!(diags[0].severity, FrontmatterSeverity::Error);
        assert!(
            diags[0].message.contains("typo_key"),
            "error message must name the unknown key"
        );
        assert!(
            !has_key(&map, "typo_key"),
            "unknown key must not appear in validated map"
        );
    }

    #[test]
    fn deterministic_on_model_produces_warning() {
        let (map, diags) = parse_frontmatter("deterministic: true", DeclarationKind::Model);
        assert_eq!(diags.len(), 1, "expected exactly one diagnostic");
        assert_eq!(diags[0].severity, FrontmatterSeverity::Warning);
        assert!(
            diags[0].message.contains("deterministic"),
            "warning must name the inapplicable key"
        );
        assert!(
            !has_key(&map, "deterministic"),
            "inapplicable key must not appear in validated map"
        );
    }

    #[test]
    fn deterministic_on_define_is_clean() {
        let (map, diags) = parse_frontmatter("deterministic: true", DeclarationKind::Define);
        assert!(
            diags.is_empty(),
            "no diagnostics expected for valid Define key"
        );
        assert!(
            has_key(&map, "deterministic"),
            "applicable key must appear in validated map"
        );
    }

    #[test]
    fn deterministic_on_extern_is_clean() {
        let (map, diags) = parse_frontmatter("deterministic: true", DeclarationKind::Extern);
        assert!(
            diags.is_empty(),
            "no diagnostics expected for valid Extern key"
        );
        assert!(has_key(&map, "deterministic"));
    }

    #[test]
    fn non_mapping_top_level_produces_error() {
        let (map, diags) = parse_frontmatter("just a scalar string", DeclarationKind::Model);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, FrontmatterSeverity::Error);
        assert!(map.is_empty());
    }

    #[test]
    fn empty_text_is_clean() {
        let (map, diags) = parse_frontmatter("", DeclarationKind::Model);
        assert!(diags.is_empty());
        assert!(map.is_empty());
    }

    #[test]
    fn null_yaml_is_clean() {
        let (map, diags) = parse_frontmatter("~", DeclarationKind::Model);
        assert!(diags.is_empty());
        assert!(map.is_empty());
    }

    #[test]
    fn whitespace_only_is_clean() {
        let (map, diags) = parse_frontmatter("   \n\t\n", DeclarationKind::Model);
        assert!(diags.is_empty());
        assert!(map.is_empty());
    }

    #[test]
    fn valid_model_keys_pass_through() {
        let yaml = "materialization: table\ntags:\n  - analytics";
        let (map, diags) = parse_frontmatter(yaml, DeclarationKind::Model);
        assert!(diags.is_empty(), "no diagnostics for valid model keys");
        assert!(has_key(&map, "materialization"));
        assert!(has_key(&map, "tags"));
    }

    #[test]
    fn model_key_on_define_produces_warning() {
        let (map, diags) = parse_frontmatter("materialization: table", DeclarationKind::Define);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, FrontmatterSeverity::Warning);
        assert!(!has_key(&map, "materialization"));
    }

    #[test]
    fn multiple_mixed_keys_produce_separate_diags() {
        // One unknown key + one inapplicable key on a Model
        let yaml = "typo_key: true\ndeterministic: true\nmaterialization: table";
        let (map, diags) = parse_frontmatter(yaml, DeclarationKind::Model);
        // typo_key → Error, deterministic → Warning, materialization → kept
        assert_eq!(
            diags.len(),
            2,
            "expected two diagnostics (one Error, one Warning)"
        );
        let has_error = diags
            .iter()
            .any(|d| d.severity == FrontmatterSeverity::Error);
        let has_warning = diags
            .iter()
            .any(|d| d.severity == FrontmatterSeverity::Warning);
        assert!(has_error, "must have an Error for the unknown key");
        assert!(has_warning, "must have a Warning for the inapplicable key");
        assert!(!has_key(&map, "typo_key"));
        assert!(!has_key(&map, "deterministic"));
        assert!(
            has_key(&map, "materialization"),
            "valid model key must be kept"
        );
    }

    #[test]
    fn tags_is_applicable_to_all_kinds() {
        for kind in [
            DeclarationKind::Model,
            DeclarationKind::Define,
            DeclarationKind::Extern,
        ] {
            let (map, diags) = parse_frontmatter("tags:\n  - foo", kind);
            assert!(diags.is_empty(), "tags must be clean for {:?}", kind);
            assert!(
                has_key(&map, "tags"),
                "tags must appear in validated map for {:?}",
                kind
            );
        }
    }

    #[test]
    fn invalid_yaml_produces_error() {
        let (map, diags) = parse_frontmatter(": bad : yaml :", DeclarationKind::Model);
        assert!(
            !diags.is_empty(),
            "invalid YAML must produce at least one diagnostic"
        );
        assert_eq!(diags[0].severity, FrontmatterSeverity::Error);
        assert!(map.is_empty());
    }
}
