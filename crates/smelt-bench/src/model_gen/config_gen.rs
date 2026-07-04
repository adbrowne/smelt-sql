use super::graph_builder::{GraphSpec, ModelSpec, ModelType, SOURCE_NAMES};

/// Generate smelt.yml content for the workspace.
pub fn generate_smelt_yml(_spec: &GraphSpec, models: &[ModelSpec]) -> String {
    let mut yml = String::from(
        "name: bench_workspace\nversion: 1\n\npaths:\n  - models\n\n\ntargets:\n  dev:\n    type: duckdb\n    database: target/bench.duckdb\n    schema: main\n\ndefault_materialization: view\n\nmodels:\n",
    );

    for model in models {
        if model.model_type == ModelType::Sql {
            yml.push_str(&format!(
                "  {}:\n    materialization: table\n    refresh: batched\n    timeseries:\n      event_time_column: event_time\n      partition_column: event_date\n      granularity: day\n",
                model.name
            ));
        }
    }

    yml
}

/// Column definitions for source tables.
const SOURCE_COLUMNS: &[(&str, &[(&str, &str)])] = &[
    (
        "users",
        &[
            ("user_id", "integer"),
            ("email_domain", "varchar"),
            ("created_at", "timestamp"),
            ("is_active", "boolean"),
            ("country", "varchar"),
            ("plan_type", "varchar"),
        ],
    ),
    (
        "events",
        &[
            ("event_id", "integer"),
            ("user_id", "integer"),
            ("event_type", "varchar"),
            ("event_time", "timestamp"),
            ("event_date", "date"),
            ("amount", "decimal"),
            ("category", "varchar"),
            ("platform", "varchar"),
            ("session_id", "varchar"),
        ],
    ),
    (
        "products",
        &[
            ("product_id", "integer"),
            ("category", "varchar"),
            ("price", "decimal"),
            ("quantity", "integer"),
            ("created_at", "timestamp"),
        ],
    ),
    (
        "orders",
        &[
            ("order_id", "integer"),
            ("user_id", "integer"),
            ("product_id", "integer"),
            ("amount", "decimal"),
            ("status", "varchar"),
            ("created_at", "timestamp"),
        ],
    ),
    (
        "sessions",
        &[
            ("session_id", "varchar"),
            ("user_id", "integer"),
            ("duration_seconds", "integer"),
            ("page_path", "varchar"),
            ("created_at", "timestamp"),
        ],
    ),
];

/// Generate sources.yml content with source table definitions.
///
/// Uses the nested object format expected by SourcesConfig's custom deserializer:
/// ```yaml
/// sources:
///   source_name:
///     tables:
///       table_name:
///         columns:
///           - name: col
///             type: INTEGER
/// ```
pub fn generate_sources_yml(spec: &GraphSpec) -> String {
    let mut yml = String::from("sources:\n  source:\n    tables:\n");

    for name in SOURCE_NAMES.iter().take(spec.num_sources) {
        // Find matching column defs or use a default set
        let columns = SOURCE_COLUMNS
            .iter()
            .find(|(n, _)| *n == *name)
            .map(|(_, cols)| *cols);

        yml.push_str(&format!("      {}:\n        columns:\n", name));

        if let Some(cols) = columns {
            for (col_name, col_type) in cols {
                yml.push_str(&format!(
                    "          - name: {}\n            type: {}\n",
                    col_name,
                    col_type.to_uppercase()
                ));
            }
        } else {
            // Default columns for tables without specific definitions
            for (col_name, col_type) in &[
                ("id", "INTEGER"),
                ("user_id", "INTEGER"),
                ("amount", "DECIMAL"),
                ("status", "VARCHAR"),
                ("created_at", "TIMESTAMP"),
                ("event_date", "DATE"),
            ] {
                yml.push_str(&format!(
                    "          - name: {}\n            type: {}\n",
                    col_name, col_type
                ));
            }
        }
    }

    yml
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_gen::graph_builder;

    #[test]
    fn test_smelt_yml_valid_yaml() {
        let spec = GraphSpec::small();
        let models = graph_builder::build_graph(&spec);
        let yml = generate_smelt_yml(&spec, &models);

        // Should parse as valid YAML
        let _: serde_yaml::Value = serde_yaml::from_str(&yml).expect("Invalid YAML");
    }

    #[test]
    fn test_sources_yml_valid_yaml() {
        let spec = GraphSpec::small();
        let yml = generate_sources_yml(&spec);

        let _: serde_yaml::Value = serde_yaml::from_str(&yml).expect("Invalid YAML");
    }
}
