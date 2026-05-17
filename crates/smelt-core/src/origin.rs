//! Model origin discrimination for generator-emitted vs hand-authored models.
//!
//! The `ModelOriginKind` enum is carried on `ExplainModel` and `CatalogModel`
//! entries to let serializers produce the `origin` field per the CLI spec
//! (`docs/specs/cli.md` §"`smelt explain --json` output schema`").
//!
//! For hand-authored models the field is absent (via `skip_serializing_if = "Option::is_none"`
//! on the containing `Option<ModelOriginKind>`). For generator-emitted models
//! the value is `Generated { generator_file, generator_name }`.

use serde::Serialize;

/// Discriminates hand-authored models from generator-emitted models.
///
/// The variant is serialized as a JSON object with a `"type"` discriminant key
/// (via `#[serde(tag = "type")]`):
///
/// ```json
/// { "type": "generated", "generator_file": "models/cohorts.gen.sql", "generator_name": "us_west" }
/// ```
///
/// Hand-authored models omit the `origin` field entirely — the containing
/// `Option<ModelOriginKind>` uses `#[serde(skip_serializing_if = "Option::is_none")]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelOriginKind {
    /// The model was emitted by a generator file.
    Generated {
        /// Workspace-relative path of the generator `.sql` file (e.g.
        /// `"models/cohorts.gen.sql"`), with `/` separators regardless of OS.
        generator_file: String,
        /// The `ModelDef.name` value that produced this emitted model.
        generator_name: String,
    },
}
