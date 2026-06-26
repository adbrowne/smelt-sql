use anyhow::Result;
use serde::Serialize;
use smelt_core::config::RefreshStrategy;
use smelt_core::graph::DependencyGraph;
use smelt_core::ModelOriginKind;
use smelt_db::ColumnSource;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use crate::Config;

/// A reference to a test model that targets a given model.
#[derive(Debug, Clone, Serialize)]
pub struct TestRef {
    pub name: String,
    pub path: String,
}

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
    /// Refresh axis: `"cumulative"` when the model uses the cumulative-aggregate
    /// merge loop. Omitted when the model uses the default full-refresh strategy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh: Option<RefreshStrategy>,
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
    /// Test models (`materialization: test`) that declare `test.model: <this model>`
    /// in their frontmatter. Omitted when no tests target this model.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tests_targeting: Vec<TestRef>,
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

/// Return a workspace-relative string for `path`, relative to `project_dir`.
/// For virtual paths containing `::` (emitted models), strips the prefix from
/// the real-file portion before the separator and re-appends the virtual suffix.
/// Falls back to the original display string when `path` is not under `project_dir`.
fn workspace_relative(path: &Path, project_dir: &Path) -> String {
    let s = path.display().to_string();
    if let Some(sep) = s.find("::") {
        let file_part = Path::new(&s[..sep]);
        let rel = file_part
            .strip_prefix(project_dir)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| s[..sep].to_string());
        return format!("{}{}", rel, &s[sep..]);
    }
    path.strip_prefix(project_dir)
        .map(|p| p.display().to_string())
        .unwrap_or(s)
}

/// Build a complete catalog from the dependency graph, config, and Salsa DB.
///
/// `origins` maps emitted model names to `(generator_file, generator_def_name)`.
/// `test_targets` maps each model name to the list of test models targeting it.
/// `project_dir` is the directory containing `smelt.yml`; all `path` fields in
/// the output are made relative to it.
pub fn build_catalog(
    graph: &DependencyGraph,
    config: &Config,
    db: &smelt_db::Database,
    origins: &HashMap<String, (String, String)>,
    test_targets: &HashMap<String, Vec<TestRef>>,
    project_dir: &Path,
    selected_names: Option<&BTreeSet<String>>,
) -> Result<Catalog> {
    let execution_order = match selected_names {
        Some(sel) => {
            let hs: HashSet<String> = sel.iter().cloned().collect();
            graph.filtered_execution_order(&hs)?
        }
        None => graph.execution_order()?,
    };

    // Build dependents (downstream) map by inverting dependencies (deduplicated).
    let mut dependents_map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (model_name, _model) in graph.iter_models() {
        for dep in graph.get_upstream(model_name) {
            dependents_map
                .entry(dep)
                .or_default()
                .insert(model_name.to_string());
        }
    }

    let mut models = BTreeMap::new();
    let mut tag_index: BTreeMap<String, Vec<String>> = BTreeMap::new();

    let ws = smelt_db::Workspace::try_get(db);

    for (model_name, model_file) in graph.iter_models() {
        if let Some(sel) = selected_names {
            if !sel.contains(model_name) {
                continue;
            }
        }
        let metadata = model_file.metadata.as_deref();
        let frontmatter = smelt_planner::Frontmatter::parse(&model_file.content);

        let materialization_enum = config.get_materialization_with_metadata(model_name, metadata);
        let materialization = serde_json::to_value(&materialization_enum)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "unknown".to_string());

        // Get typed schema from Salsa DB
        let schema = match (ws, db.source_file(&model_file.path)) {
            (Some(w), Some(f)) => smelt_db::typed_model_schema(db, w, f),
            _ => std::sync::Arc::new(smelt_db::ModelSchema::empty()),
        };

        // Get frontmatter column metadata for enrichment
        let frontmatter_columns = metadata.map(|m| m.columns.clone()).unwrap_or_default();

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

        let upstream: Vec<String> = {
            let mut seen = BTreeSet::new();
            graph
                .get_upstream(model_name)
                .into_iter()
                .filter(|dep| seen.insert(dep.clone()))
                .collect()
        };

        let downstream: Vec<String> = dependents_map
            .get(model_name)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();

        let inc_config = config
            .get_incremental_with_metadata(model_name, metadata)
            .cloned()
            .or_else(|| frontmatter.as_ref().and_then(|f| f.incremental.clone()));
        let ts_config = config
            .get_timeseries_with_metadata(model_name, metadata)
            .cloned()
            .or_else(|| metadata.and_then(|m| m.timeseries.clone()));

        let incremental = match (inc_config, ts_config) {
            (Some(inc), Some(ts)) => Some(CatalogIncremental {
                granularity: serde_json::to_value(&ts.granularity)
                    .ok()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_else(|| "unknown".to_string()),
                partition_column: ts.partition_column.clone(),
                event_time_column: ts.event_time_column.clone(),
                unique_key: inc.unique_key.clone(),
            }),
            _ => None,
        };

        let tags = config.get_tags(model_name, metadata);
        // Build tag index
        for tag in &tags {
            tag_index
                .entry(tag.clone())
                .or_default()
                .push(model_name.to_string());
        }

        let description = metadata.and_then(|m| m.description.clone());
        let owner = metadata.and_then(|m| m.owner.clone());

        // Build origin for generator-emitted models from the provenance map.
        let origin = origins
            .get(model_name)
            .map(|(gf, gn)| ModelOriginKind::Generated {
                generator_file: workspace_relative(Path::new(gf), project_dir),
                generator_name: gn.clone(),
            });

        let tests_targeting = test_targets.get(model_name).cloned().unwrap_or_default();

        // Emit `refresh: "cumulative"` when the model is cumulative; omit otherwise.
        let refresh = metadata
            .and_then(|m| m.refresh.clone())
            .filter(|r| *r == RefreshStrategy::Cumulative);

        models.insert(
            model_name.to_string(),
            CatalogModel {
                name: model_name.to_string(),
                description,
                owner,
                tags,
                materialization,
                refresh,
                path: workspace_relative(&model_file.path, project_dir),
                columns,
                upstream,
                downstream,
                incremental,
                origin,
                tests_targeting,
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
