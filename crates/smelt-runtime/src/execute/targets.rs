use std::collections::HashMap;

use smelt_core::config::Config;

/// The maintenance-statement dialect for a target, derived from its declared
/// backend type — the no-backend equivalent of
/// `smelt_backend::maintenance_dialect(backend.dialect())`, so `--dry-run`
/// (which never opens a connection) still renders statements in the target's
/// own dialect (`docs/specs/cli.md` §"`--dry-run` prints the maintenance
/// statements"). Falls back to DuckDb for an unrecognised target.
pub(crate) fn maintenance_dialect_for_target(
    config: &Config,
    target: &str,
) -> smelt_logical::maintenance::emit::MaintenanceDialect {
    config
        .targets
        .get(target)
        .and_then(|t| t.backend_type().ok())
        .map(|bt| match bt {
            smelt_core::config::BackendType::DuckDB => smelt_backend::SqlDialect::DuckDB,
            smelt_core::config::BackendType::Spark => smelt_backend::SqlDialect::SparkSQL,
            smelt_core::config::BackendType::BigQuery => smelt_backend::SqlDialect::BigQuery,
        })
        .map(smelt_backend::maintenance_dialect)
        .unwrap_or(smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb)
}

/// `target`'s declared [`smelt_backend::SqlDialect`], purely from `Config`
/// (no live backend needed) — the same `BackendType` match
/// [`maintenance_dialect_for_target`] uses, stopping one step earlier so
/// availability resolution (which needs `SqlDialect`, not
/// `MaintenanceDialect`) can share it.
pub(crate) fn sql_dialect_for_target(config: &Config, target: &str) -> smelt_backend::SqlDialect {
    config
        .targets
        .get(target)
        .and_then(|t| t.backend_type().ok())
        .map(|bt| match bt {
            smelt_core::config::BackendType::DuckDB => smelt_backend::SqlDialect::DuckDB,
            smelt_core::config::BackendType::Spark => smelt_backend::SqlDialect::SparkSQL,
            smelt_core::config::BackendType::BigQuery => smelt_backend::SqlDialect::BigQuery,
        })
        .unwrap_or(smelt_backend::SqlDialect::DuckDB)
}

/// `state_availability`'s value for `target`, falling back to a fresh
/// (pure, cheap) derivation for a target the run's own
/// `target_assignments` did not cover — defensive only; every production
/// call site's `target` is itself sourced from `target_assignments` or
/// `config.get_target`, which agree by construction.
pub(crate) fn availability_for_target(
    state_availability: &HashMap<
        String,
        smelt_logical::maintenance::availability::StateAvailability,
    >,
    target: &str,
    config: &Config,
) -> smelt_logical::maintenance::availability::StateAvailability {
    state_availability.get(target).cloned().unwrap_or_else(|| {
        crate::maintenance_availability::availability_for_run(
            sql_dialect_for_target(config, target),
            config,
        )
    })
}
