//! Selector parsing for model filtering.
//!
//! Supports dbt-style selector syntax:
//! - `model_name` — select a specific model
//! - `tag:revenue` — select models with a tag
//! - `+tag:revenue` — select matching models + upstream dependencies
//! - `tag:revenue+` — select matching models + downstream dependents
//! - `+tag:revenue+` — select matching models + both directions

use std::fmt;

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
}

impl fmt::Display for Selector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.include_upstream {
            write!(f, "+")?;
        }
        match &self.method {
            SelectionMethod::ModelName(name) => write!(f, "{}", name)?,
            SelectionMethod::Tag(tag) => write!(f, "tag:{}", tag)?,
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
    InvalidCharacter(char),
}

impl fmt::Display for SelectorParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SelectorParseError::Empty => write!(f, "Selector cannot be empty"),
            SelectorParseError::EmptyTag => {
                write!(f, "Tag name cannot be empty in 'tag:' selector")
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
}
