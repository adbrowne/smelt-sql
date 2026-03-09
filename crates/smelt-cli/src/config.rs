// Re-export all config types from smelt-core
pub use smelt_core::{
    BackendType, Config, IncrementalConfig, Materialization, ModelConfig, Target,
};

// Re-export project discovery
pub use smelt_core::find_project_root;

// Re-export sources
pub use smelt_core::{SourceColumnDef, SourceDef, SourceTableDef, SourcesConfig};
