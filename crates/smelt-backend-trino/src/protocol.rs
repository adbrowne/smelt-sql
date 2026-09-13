//! Serde types for Trino's `/v1/statement` JSON protocol
//! (<https://trino.io/docs/current/develop/client-protocol.html>).
//!
//! These types are deliberately tolerant of unknown fields (no
//! `#[serde(deny_unknown_fields)]`) — Trino's protocol carries many fields
//! the client does not read (warnings, `updateType`, statistics), and a
//! coordinator version bump adding a field must not break deserialization.

use serde::Deserialize;

/// One page of a Trino query's results, returned by the initial
/// `POST /v1/statement` and every subsequent `GET nextUri`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryResults {
    #[serde(default)]
    pub id: String,
    /// Present until the results are fully drained; its absence is the only
    /// correct "no more pages" signal (a page can be `QUEUED` — no
    /// `columns`/`data` yet — while still carrying a `nextUri`).
    #[serde(default)]
    pub next_uri: Option<String>,
    #[serde(default)]
    pub columns: Option<Vec<Column>>,
    #[serde(default)]
    pub data: Option<Vec<Vec<serde_json::Value>>>,
    pub stats: StatementStats,
    #[serde(default)]
    pub error: Option<QueryError>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    pub name: String,
    /// The raw type spelling as Trino prints it, e.g. `"decimal(18,4)"` or
    /// `"varchar(32)"`. This is what `arrow_convert` parses; `type_signature`
    /// carries the same information structured, and is modeled for
    /// protocol fidelity even though the client reads the string form.
    #[serde(rename = "type")]
    pub raw_type: String,
    #[serde(default)]
    pub type_signature: Option<ClientTypeSignature>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientTypeSignature {
    pub raw_type: String,
    #[serde(default)]
    pub arguments: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatementStats {
    pub state: String,
}

/// Trino's error object, present on a failed page.
///
/// A page carrying `error` is never a valid result, even if it also carries
/// `columns`/`data` (an empty `data: []` alongside a set `error` is the
/// documented failure shape, not a zero-row success).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryError {
    pub message: String,
    #[serde(default)]
    pub error_code: i64,
    #[serde(default)]
    pub error_name: String,
    #[serde(default)]
    pub error_type: String,
    #[serde(default)]
    pub error_location: Option<ErrorLocation>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorLocation {
    pub line_number: i64,
    pub column_number: i64,
}
