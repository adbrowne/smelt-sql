//! Custom LSP notification types for smelt editor integration.

use serde::{Deserialize, Serialize};

/// A single test model discovered in the workspace.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TestInfo {
    /// Test model name (used as `smelt test --select <name>`).
    pub name: String,
    /// File URI of the test file.
    pub uri: String,
    /// Start line of the model within the file (0-based).
    pub line: u32,
}

/// Params for the `smelt/publishTests` notification.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublishTestsParams {
    pub tests: Vec<TestInfo>,
}

/// `smelt/publishTests` — sent after workspace init and on file changes.
///
/// The VSCode TestController subscribes to this and rebuilds its test tree
/// rather than scanning files with regex.
pub enum PublishTests {}

impl tower_lsp::lsp_types::notification::Notification for PublishTests {
    const METHOD: &'static str = "smelt/publishTests";
    type Params = PublishTestsParams;
}
