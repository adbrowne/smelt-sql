//! Trino client configuration.

use std::fmt;

/// Connection details for a Trino coordinator.
///
/// `Debug` is hand-written rather than derived so that `password` can never
/// surface in a log line, panic message, or diagnostic — the connection
/// security rule in `docs/specs/multi_backend.md` §"Connection security".
#[derive(Clone, Default)]
pub struct TrinoClientConfig {
    pub base_url: String,
    pub user: String,
    pub catalog: String,
    pub schema: String,
    pub password: Option<String>,
}

impl fmt::Debug for TrinoClientConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrinoClientConfig")
            .field("base_url", &self.base_url)
            .field("user", &self.user)
            .field("catalog", &self.catalog)
            .field("schema", &self.schema)
            .field("password", &self.password.as_ref().map(|_| "REDACTED"))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_prints_the_password() {
        let config = TrinoClientConfig {
            password: Some("hunter2".to_string()),
            ..Default::default()
        };
        let debug_output = format!("{config:?}");
        assert!(debug_output.contains("REDACTED"));
        assert!(!debug_output.contains("hunter2"));
    }
}
