use std::fmt;
use std::path::{Path, PathBuf};

/// Canonical identifier for a model.
///
/// For single-model files, the model name comes from the filename stem.
/// For multi-model files, the model name comes from the `--- name: foo ---` delimiter.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModelId {
    /// Physical file path on disk.
    pub path: PathBuf,
    /// Model name.
    pub model_name: String,
    /// Whether this model came from a multi-model file.
    pub is_multi_model: bool,
}

impl ModelId {
    /// Create a ModelId for a single-model file (name derived from filename).
    pub fn from_path(path: PathBuf) -> Self {
        let model_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        Self {
            path,
            model_name,
            is_multi_model: false,
        }
    }

    /// Create a ModelId for a model within a multi-model file.
    pub fn multi_model(path: PathBuf, model_name: String) -> Self {
        Self {
            path,
            model_name,
            is_multi_model: true,
        }
    }

    /// Return the path to use as a Salsa query key.
    ///
    /// For single-model files, this is the file path as-is.
    /// For multi-model files, this appends `::model_name` to make a unique virtual path.
    pub fn salsa_key(&self) -> PathBuf {
        if self.is_multi_model {
            PathBuf::from(format!("{}::{}", self.path.display(), self.model_name))
        } else {
            self.path.clone()
        }
    }

    /// Return the physical source path (always the real file on disk).
    pub fn source_path(&self) -> &Path {
        &self.path
    }
}

impl fmt::Display for ModelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_multi_model {
            write!(f, "{}::{}", self.path.display(), self.model_name)
        } else {
            write!(f, "{}", self.model_name)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_model_id() {
        let id = ModelId::from_path(PathBuf::from("models/users.sql"));
        assert_eq!(id.model_name, "users");
        assert!(!id.is_multi_model);
        assert_eq!(id.salsa_key(), PathBuf::from("models/users.sql"));
    }

    #[test]
    fn test_multi_model_id() {
        let id = ModelId::multi_model(
            PathBuf::from("models/pipeline.sql"),
            "staging_events".to_string(),
        );
        assert_eq!(id.model_name, "staging_events");
        assert!(id.is_multi_model);
        assert_eq!(
            id.salsa_key(),
            PathBuf::from("models/pipeline.sql::staging_events")
        );
    }

    #[test]
    fn test_display() {
        let single = ModelId::from_path(PathBuf::from("models/users.sql"));
        assert_eq!(format!("{}", single), "users");

        let multi =
            ModelId::multi_model(PathBuf::from("models/pipeline.sql"), "staging".to_string());
        assert_eq!(format!("{}", multi), "models/pipeline.sql::staging");
    }
}
