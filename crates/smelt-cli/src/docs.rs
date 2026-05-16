use anyhow::Result;
use serde::Serialize;
use smelt_core::ModelOriginKind;
use smelt_db::ColumnSource;
use std::collections::{BTreeMap, BTreeSet};

use crate::logical_graph::LogicalGraph;
use crate::Config;

/// Complete project catalog — the top-level output of `smelt docs generate`.
#[derive(Debug, Serialize)]
pub struct Catalog {
    pub project: ProjectSummary,
    pub models: BTreeMap<String, CatalogModel>,
    pub execution_order: Vec<String>,
    pub tag_index: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct ProjectSummary {
    pub name: String,
    pub model_count: usize,
    pub generated_at: String,
}

#[derive(Debug, Serialize)]
pub struct CatalogModel {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    pub materialization: String,
    pub path: String,
    pub columns: Vec<CatalogColumn>,
    pub upstream: Vec<String>,
    pub downstream: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incremental: Option<CatalogIncremental>,
    /// For generator-emitted models: provenance identifying the generator file
    /// and the `ModelDef.name` that produced this model. Omitted for hand-authored
    /// models.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<ModelOriginKind>,
}

#[derive(Debug, Serialize)]
pub struct CatalogColumn {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nullable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tests: Vec<String>,
    pub expression: String,
    pub source: CatalogColumnSource,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum CatalogColumnSource {
    #[serde(rename = "computed")]
    Computed,
    #[serde(rename = "from_model")]
    FromModel { model: String, column: String },
    #[serde(rename = "wildcard")]
    Wildcard { model: String },
    #[serde(rename = "external_table")]
    ExternalTable { table: String },
    #[serde(rename = "unknown")]
    Unknown,
}

#[derive(Debug, Serialize)]
pub struct CatalogIncremental {
    pub granularity: String,
    pub partition_column: String,
    pub event_time_column: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unique_key: Vec<String>,
}

/// Build a complete catalog from the logical graph and Salsa DB.
pub fn build_catalog(
    graph: &LogicalGraph,
    config: &Config,
    db: &smelt_db::Database,
) -> Result<Catalog> {
    let execution_order = graph.execution_order()?;

    // Build dependents (downstream) map by inverting dependencies (deduplicated)
    let mut dependents_map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for node in graph.iter_nodes() {
        for dep in &node.dependencies {
            if graph.get_node(dep).is_ok() {
                dependents_map
                    .entry(dep.clone())
                    .or_default()
                    .insert(node.name.clone());
            }
        }
    }

    let mut models = BTreeMap::new();
    let mut tag_index: BTreeMap<String, Vec<String>> = BTreeMap::new();

    let ws = smelt_db::Workspace::try_get(db);

    for node in graph.iter_nodes() {
        // Get typed schema from Salsa DB
        let schema = match (ws, db.source_file(&node.model_file.path)) {
            (Some(w), Some(f)) => smelt_db::typed_model_schema(db, w, f),
            _ => std::sync::Arc::new(smelt_db::ModelSchema::empty()),
        };

        // Get frontmatter column metadata for enrichment
        let frontmatter_columns = node
            .model_file
            .metadata
            .as_ref()
            .map(|m| &m.columns)
            .cloned()
            .unwrap_or_default();

        let columns: Vec<CatalogColumn> = schema
            .columns
            .iter()
            .filter(|c| c.name != "*")
            .map(|c| {
                let (data_type, nullable) = match &c.data_type {
                    Some(tc) => (Some(tc.data_type.to_sql()), Some(tc.nullable)),
                    None => (None, None),
                };

                let source = match &c.source {
                    ColumnSource::Computed => CatalogColumnSource::Computed,
                    ColumnSource::FromModel {
                        model_name,
                        column_name,
                    } => CatalogColumnSource::FromModel {
                        model: model_name.clone(),
                        column: column_name.clone(),
                    },
                    ColumnSource::Wildcard { model_name } => CatalogColumnSource::Wildcard {
                        model: model_name.clone(),
                    },
                    ColumnSource::ExternalTable { table_name } => {
                        CatalogColumnSource::ExternalTable {
                            table: table_name.clone(),
                        }
                    }
                    ColumnSource::Unknown => CatalogColumnSource::Unknown,
                };

                // Enrich with frontmatter metadata
                let fm = frontmatter_columns.get(&c.name);
                let description = fm.and_then(|m| m.description.clone());
                let tests = fm
                    .map(|m| {
                        m.tests
                            .iter()
                            .map(|t| match t {
                                smelt_core::metadata::ColumnTest::Simple(s) => s.clone(),
                                smelt_core::metadata::ColumnTest::Parameterized(map) => {
                                    map.keys().cloned().collect::<Vec<_>>().join(", ")
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                CatalogColumn {
                    name: c.name.clone(),
                    data_type,
                    nullable,
                    description,
                    tests,
                    expression: c.expression.clone(),
                    source,
                }
            })
            .collect();

        let materialization = serde_json::to_value(&node.materialization)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "unknown".to_string());

        let upstream: Vec<String> = {
            let mut seen = BTreeSet::new();
            node.dependencies
                .iter()
                .filter(|dep| graph.get_node(dep).is_ok() && seen.insert((*dep).clone()))
                .cloned()
                .collect()
        };

        let downstream: Vec<String> = dependents_map
            .get(&node.name)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();

        let incremental = node.incremental.as_ref().map(|inc| CatalogIncremental {
            granularity: serde_json::to_value(&inc.granularity)
                .ok()
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_else(|| "unknown".to_string()),
            partition_column: inc.partition_column.clone(),
            event_time_column: inc.event_time_column.clone(),
            unique_key: inc.unique_key.clone(),
        });

        // Build tag index
        for tag in &node.tags {
            tag_index
                .entry(tag.clone())
                .or_default()
                .push(node.name.clone());
        }

        let description = node
            .model_file
            .metadata
            .as_ref()
            .and_then(|m| m.description.clone());

        let owner = node
            .model_file
            .metadata
            .as_ref()
            .and_then(|m| m.owner.clone());

        // Build origin for generator-emitted models.
        let origin = match (&node.generator_file, &node.generator_name) {
            (Some(gf), Some(gn)) => Some(ModelOriginKind::Generated {
                generator_file: gf.clone(),
                generator_name: gn.clone(),
            }),
            _ => None,
        };

        models.insert(
            node.name.clone(),
            CatalogModel {
                name: node.name.clone(),
                description,
                owner,
                tags: node.tags.clone(),
                materialization,
                path: node.model_file.path.display().to_string(),
                columns,
                upstream,
                downstream,
                incremental,
                origin,
            },
        );
    }

    // Sort tag index entries
    for names in tag_index.values_mut() {
        names.sort();
    }

    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    Ok(Catalog {
        project: ProjectSummary {
            name: config.name.clone(),
            model_count: models.len(),
            generated_at: now,
        },
        models,
        execution_order,
        tag_index,
    })
}
