//! Trino error-object -> typed `BackendError` mapping, and transport-level
//! failure classification.
//!
//! Every arm is explicit; there is no stringly catch-all that swallows an
//! unrecognised Trino error code into a generic `Other`
//! (`CLAUDE.md` §"Fail-loud discipline").

use smelt_backend::BackendError;

use crate::protocol::QueryError;

/// Map a Trino `QueryError` object to the `BackendError` variant that best
/// describes it. The message Trino sent is always preserved.
pub fn map_trino_error(err: &QueryError) -> BackendError {
    match err.error_name.as_str() {
        "TABLE_NOT_FOUND" => BackendError::not_found(String::new(), err.message.clone()),
        "SCHEMA_NOT_FOUND" => BackendError::SchemaNotFound {
            schema: err.message.clone(),
        },
        "NOT_SUPPORTED" | "FUNCTION_NOT_FOUND" => {
            BackendError::unsupported("trino", err.message.clone())
        }
        _ => BackendError::execution_failed("trino", err.message.clone()),
    }
}

/// Classify a `reqwest` transport-level failure (connection refused, DNS
/// failure, timeout before any HTTP response) as `ConnectionFailed`, versus
/// a failure that occurred after a response was received.
pub fn map_transport_error(err: &reqwest::Error) -> BackendError {
    BackendError::connection_failed(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::QueryError;

    fn error(name: &str, message: &str) -> QueryError {
        QueryError {
            message: message.to_string(),
            error_code: 0,
            error_name: name.to_string(),
            error_type: "USER_ERROR".to_string(),
            error_location: None,
        }
    }

    #[test]
    fn trino_error_codes_map_to_typed_variants() {
        assert!(matches!(
            map_trino_error(&error("TABLE_NOT_FOUND", "table foo.bar not found")),
            BackendError::NotFound { .. }
        ));
        assert!(matches!(
            map_trino_error(&error("SCHEMA_NOT_FOUND", "schema foo not found")),
            BackendError::SchemaNotFound { .. }
        ));
        assert!(matches!(
            map_trino_error(&error("NOT_SUPPORTED", "MERGE not supported")),
            BackendError::UnsupportedFeature { .. }
        ));
        assert!(matches!(
            map_trino_error(&error("FUNCTION_NOT_FOUND", "no such function foo")),
            BackendError::UnsupportedFeature { .. }
        ));

        let generic = map_trino_error(&error("GENERIC_INTERNAL_ERROR", "boom"));
        match generic {
            BackendError::ExecutionFailed { message, .. } => assert_eq!(message, "boom"),
            other => panic!("expected ExecutionFailed, got {other:?}"),
        }

        let user = map_trino_error(&error("SYNTAX_ERROR", "line 1: mismatched input"));
        match user {
            BackendError::ExecutionFailed { message, .. } => {
                assert_eq!(message, "line 1: mismatched input")
            }
            other => panic!("expected ExecutionFailed, got {other:?}"),
        }
    }
}
