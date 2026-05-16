//! Selector parsing for model filtering.
//!
//! Supports dbt-style selector syntax:
//! - `model_name` — select a specific model
//! - `tag:revenue` — select models with a tag
//! - `+tag:revenue` — select matching models + upstream dependencies
//! - `tag:revenue+` — select matching models + downstream dependents
//! - `+tag:revenue+` — select matching models + both directions
//! - `generator_file:models/cohorts.gen.sql` — select all emitted models from
//!   the given generator file (excluding collision losers)

use std::fmt;
use std::path::PathBuf;

/// A parsed selector that identifies which models to include.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector {
    pub method: SelectionMethod,
    pub include_upstream: bool,
    pub include_downstream: bool,
}

/// How to match models.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionMethod {
    /// Select a model by name
    ModelName(String),
    /// Select models by tag
    Tag(String),
    /// Select all emitted models from a generator file (by workspace-relative
    /// path). Excludes collision losers — only surviving emitted models are
    /// matched. A path pointing at a hand-authored `.sql` file or a missing
    /// file returns an empty match set (no error).
    GeneratorFile {
        /// Workspace-relative path of the generator `.sql` file, e.g.
        /// `PathBuf::from("models/cohorts.gen.sql")`.
        path: PathBuf,
    },
}

impl SelectionMethod {
    /// Return the model name if this is a `ModelName` selector.
    pub fn model_name(&self) -> Option<&str> {
        match self {
            SelectionMethod::ModelName(name) => Some(name),
            SelectionMethod::Tag(_) => None,
            SelectionMethod::GeneratorFile { .. } => None,
        }
    }
}

impl fmt::Display for Selector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.include_upstream {
            write!(f, "+")?;
        }
        match &self.method {
            SelectionMethod::ModelName(name) => write!(f, "{}", name)?,
            SelectionMethod::Tag(tag) => write!(f, "tag:{}", tag)?,
            SelectionMethod::GeneratorFile { path } => {
                write!(f, "generator_file:{}", path.display())?;
            }
        }
        if self.include_downstream {
            write!(f, "+")?;
        }
        Ok(())
    }
}

/// Parse a selector string into a `Selector`.
///
/// # Examples
/// - `"daily_revenue"` → ModelName("daily_revenue"), no directions
/// - `"tag:revenue"` → Tag("revenue"), no directions
/// - `"+tag:revenue"` → Tag("revenue"), upstream
/// - `"tag:revenue+"` → Tag("revenue"), downstream
/// - `"+tag:revenue+"` → Tag("revenue"), both
/// - `"+daily_revenue"` → ModelName("daily_revenue"), upstream
pub fn parse_selector(input: &str) -> Result<Selector, SelectorParseError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(SelectorParseError::Empty);
    }

    // Check for upstream prefix
    let (include_upstream, rest) = if let Some(stripped) = input.strip_prefix('+') {
        (true, stripped)
    } else {
        (false, input)
    };

    // Check for downstream suffix
    let (include_downstream, rest) = if let Some(stripped) = rest.strip_suffix('+') {
        (true, stripped)
    } else {
        (false, rest)
    };

    if rest.is_empty() {
        return Err(SelectorParseError::Empty);
    }

    // Reject remaining `+` characters (e.g. "model++" would leave "model+" after stripping)
    if rest.contains('+') {
        return Err(SelectorParseError::InvalidCharacter('+'));
    }

    // Parse method
    let method = if let Some(tag) = rest.strip_prefix("tag:") {
        if tag.is_empty() {
            return Err(SelectorParseError::EmptyTag);
        }
        SelectionMethod::Tag(tag.to_string())
    } else if let Some(path_str) = rest.strip_prefix("generator_file:") {
        if path_str.is_empty() {
            return Err(SelectorParseError::EmptyPath);
        }
        SelectionMethod::GeneratorFile {
            path: PathBuf::from(path_str),
        }
    } else {
        SelectionMethod::ModelName(rest.to_string())
    };

    Ok(Selector {
        method,
        include_upstream,
        include_downstream,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SelectorParseError {
    Empty,
    EmptyTag,
    /// `generator_file:` prefix with no path component.
    EmptyPath,
    InvalidCharacter(char),
}

impl fmt::Display for SelectorParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SelectorParseError::Empty => write!(f, "Selector cannot be empty"),
            SelectorParseError::EmptyTag => {
                write!(f, "Tag name cannot be empty in 'tag:' selector")
            }
            SelectorParseError::EmptyPath => {
                write!(f, "Path cannot be empty in 'generator_file:' selector")
            }
            SelectorParseError::InvalidCharacter(c) => {
                write!(f, "Invalid character '{}' in selector", c)
            }
        }
    }
}

impl std::error::Error for SelectorParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_name() {
        let s = parse_selector("daily_revenue").unwrap();
        assert_eq!(
            s.method,
            SelectionMethod::ModelName("daily_revenue".to_string())
        );
        assert!(!s.include_upstream);
        assert!(!s.include_downstream);
    }

    #[test]
    fn test_tag() {
        let s = parse_selector("tag:revenue").unwrap();
        assert_eq!(s.method, SelectionMethod::Tag("revenue".to_string()));
        assert!(!s.include_upstream);
        assert!(!s.include_downstream);
    }

    #[test]
    fn test_upstream_tag() {
        let s = parse_selector("+tag:revenue").unwrap();
        assert_eq!(s.method, SelectionMethod::Tag("revenue".to_string()));
        assert!(s.include_upstream);
        assert!(!s.include_downstream);
    }

    #[test]
    fn test_downstream_tag() {
        let s = parse_selector("tag:revenue+").unwrap();
        assert_eq!(s.method, SelectionMethod::Tag("revenue".to_string()));
        assert!(!s.include_upstream);
        assert!(s.include_downstream);
    }

    #[test]
    fn test_both_directions_tag() {
        let s = parse_selector("+tag:revenue+").unwrap();
        assert_eq!(s.method, SelectionMethod::Tag("revenue".to_string()));
        assert!(s.include_upstream);
        assert!(s.include_downstream);
    }

    #[test]
    fn test_upstream_model() {
        let s = parse_selector("+daily_revenue").unwrap();
        assert_eq!(
            s.method,
            SelectionMethod::ModelName("daily_revenue".to_string())
        );
        assert!(s.include_upstream);
        assert!(!s.include_downstream);
    }

    #[test]
    fn test_downstream_model() {
        let s = parse_selector("daily_revenue+").unwrap();
        assert_eq!(
            s.method,
            SelectionMethod::ModelName("daily_revenue".to_string())
        );
        assert!(!s.include_upstream);
        assert!(s.include_downstream);
    }

    #[test]
    fn test_empty_selector() {
        assert_eq!(parse_selector(""), Err(SelectorParseError::Empty));
    }

    #[test]
    fn test_empty_tag() {
        assert_eq!(parse_selector("tag:"), Err(SelectorParseError::EmptyTag));
    }

    #[test]
    fn test_just_plus() {
        assert_eq!(parse_selector("+"), Err(SelectorParseError::Empty));
    }

    #[test]
    fn test_double_plus_model() {
        assert_eq!(
            parse_selector("model_name++"),
            Err(SelectorParseError::InvalidCharacter('+'))
        );
    }

    #[test]
    fn test_double_plus_tag() {
        assert_eq!(
            parse_selector("tag:revenue++"),
            Err(SelectorParseError::InvalidCharacter('+'))
        );
    }

    #[test]
    fn test_plus_in_middle() {
        assert_eq!(
            parse_selector("model+name"),
            Err(SelectorParseError::InvalidCharacter('+'))
        );
    }

    #[test]
    fn test_display() {
        assert_eq!(
            parse_selector("+tag:revenue+").unwrap().to_string(),
            "+tag:revenue+"
        );
        assert_eq!(
            parse_selector("daily_revenue").unwrap().to_string(),
            "daily_revenue"
        );
    }

    // ── Phase 5 (E2): generator_file: selector tests ─────────────────────────

    /// `generator_file:models/cohorts.gen.sql` parses to
    /// `SelectionMethod::GeneratorFile { path: "models/cohorts.gen.sql" }`
    /// with both upstream/downstream false.
    #[test]
    fn generator_file_selector_parses_workspace_relative_path() {
        use std::path::PathBuf;

        let s = parse_selector("generator_file:models/cohorts.gen.sql").unwrap();
        match &s.method {
            SelectionMethod::GeneratorFile { path } => {
                assert_eq!(path, &PathBuf::from("models/cohorts.gen.sql"));
            }
            other => panic!("expected GeneratorFile selector, got {:?}", other),
        }
        assert!(!s.include_upstream);
        assert!(!s.include_downstream);

        // With both modifiers.
        let s2 = parse_selector("+generator_file:models/foo.gen.sql+").unwrap();
        match &s2.method {
            SelectionMethod::GeneratorFile { path } => {
                assert_eq!(path, &PathBuf::from("models/foo.gen.sql"));
            }
            other => panic!("expected GeneratorFile selector, got {:?}", other),
        }
        assert!(s2.include_upstream);
        assert!(s2.include_downstream);
    }

    /// `generator_file:` (empty path) returns `SelectorParseError::EmptyPath`.
    #[test]
    fn generator_file_selector_with_empty_path_emits_parse_error() {
        let result = parse_selector("generator_file:");
        assert_eq!(result, Err(SelectorParseError::EmptyPath));
    }
}
