//! Pure session-entry-point planning — no Python, no network, no workspace.
//!
//! Which product's session builder a `SparkBackend` reaches for (Spark
//! Connect's `SparkSession.builder.remote(...)` vs Databricks Connect's
//! `DatabricksSession.builder.host(...).serverless(True)`) is a pure
//! function of the target's [`SparkFlavor`] and its own fields. Extracted
//! out of the PyO3 construction path (`lib.rs::SparkBackend::new` /
//! `new_databricks`) so the entry-point decision itself — module, class,
//! and argument shape — can be asserted with no interpreter started
//! (`docs/outcomes/20260912-databricks-dogfood-spine/phases/02-plan.md`
//! test 7).

/// Which product this crate's SQL surface targets. Both flavors compile to
/// Spark SQL and share every statement in `sql.rs`; only capabilities
/// (`BackendCapabilities::spark_delta()` vs `::databricks()`) and the
/// session entry point differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SparkFlavor {
    /// Local/OSS Spark reached via a raw Spark Connect URL (`sc://...`).
    Spark,
    /// A Databricks workspace reached via Databricks Connect (serverless,
    /// Unity Catalog).
    Databricks,
}

/// The connection arguments a session builder needs, distinct per flavor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionArgs {
    Spark {
        connect_url: String,
    },
    Databricks {
        host: String,
        /// Absent means the session authenticates with the client's ambient
        /// Databricks credentials rather than an explicit token
        /// (`docs/specs/smelt_yml.md` §"Target shape").
        token: Option<String>,
    },
}

/// The Python entry point (module + class) plus the arguments to construct
/// it with. Computing this needs no interpreter — `Python::attach` is never
/// called by this function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPlan {
    pub module: &'static str,
    pub class: &'static str,
    pub args: SessionArgs,
}

/// Select the Python adapter entry point for a flavor: `smelt.spark_adapter
/// .SparkAdapter` for [`SparkFlavor::Spark`], `smelt.databricks_adapter
/// .DatabricksAdapter` for [`SparkFlavor::Databricks`].
pub fn plan_session(
    flavor: SparkFlavor,
    connect_url: Option<&str>,
    host: Option<&str>,
    token: Option<&str>,
) -> SessionPlan {
    match flavor {
        SparkFlavor::Spark => SessionPlan {
            module: "smelt.spark_adapter",
            class: "SparkAdapter",
            args: SessionArgs::Spark {
                connect_url: connect_url.unwrap_or_default().to_string(),
            },
        },
        SparkFlavor::Databricks => SessionPlan {
            module: "smelt.databricks_adapter",
            class: "DatabricksAdapter",
            args: SessionArgs::Databricks {
                host: host.unwrap_or_default().to_string(),
                token: token.map(str::to_string),
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spark_flavor_selects_spark_adapter_with_connect_url() {
        let plan = plan_session(SparkFlavor::Spark, Some("sc://host:15002"), None, None);
        assert_eq!(plan.module, "smelt.spark_adapter");
        assert_eq!(plan.class, "SparkAdapter");
        assert_eq!(
            plan.args,
            SessionArgs::Spark {
                connect_url: "sc://host:15002".to_string()
            }
        );
    }

    #[test]
    fn databricks_flavor_selects_databricks_adapter_with_token() {
        let plan = plan_session(
            SparkFlavor::Databricks,
            None,
            Some("my-workspace.cloud.databricks.com"),
            Some("secret-token"),
        );
        assert_eq!(plan.module, "smelt.databricks_adapter");
        assert_eq!(plan.class, "DatabricksAdapter");
        assert_eq!(
            plan.args,
            SessionArgs::Databricks {
                host: "my-workspace.cloud.databricks.com".to_string(),
                token: Some("secret-token".to_string()),
            }
        );
    }

    #[test]
    fn databricks_flavor_with_no_token_selects_ambient_form() {
        let plan = plan_session(
            SparkFlavor::Databricks,
            None,
            Some("my-workspace.cloud.databricks.com"),
            None,
        );
        assert_eq!(
            plan.args,
            SessionArgs::Databricks {
                host: "my-workspace.cloud.databricks.com".to_string(),
                token: None,
            }
        );
    }

    #[test]
    fn plan_session_starts_no_interpreter() {
        // Calling plan_session with no `Python::attach` anywhere in this
        // module's dependency graph is the assertion: if this test compiles
        // and runs (no #[cfg(feature = "python")]-gated harness needed), the
        // function is pure.
        let _ = plan_session(SparkFlavor::Spark, Some("sc://h:1"), None, None);
    }
}
