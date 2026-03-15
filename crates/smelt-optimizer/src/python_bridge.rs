//! PyO3 bridge for running Python optimizer rules.
//!
//! Discovers rules registered via `smelt.optimizer_rules` entry points,
//! converts Rust types to Python `smelt_sdk` dataclasses, calls
//! `detect`/`rewrite` on each rule, and converts results back to Rust.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use crate::analysis::{analyze_select, SelectAnalysis, SelectItemKind};
use crate::graph::ModelInfo;
use crate::types::{ExecutionStep, Transformation};

/// Discover all Python optimizer rules registered via entry points.
///
/// Calls `importlib.metadata.entry_points(group="smelt.optimizer_rules")`,
/// loads each entry point, and instantiates the rule class.
fn discover_python_rules(py: Python<'_>) -> PyResult<Vec<PyObject>> {
    let locals = PyDict::new(py);
    py.run(
        pyo3::ffi::c_str!(
            "import importlib.metadata\n\
             rules = []\n\
             try:\n\
             \teps = importlib.metadata.entry_points(group='smelt.optimizer_rules')\n\
             \tfor ep in eps:\n\
             \t\tcls = ep.load()\n\
             \t\trules.append(cls())\n\
             except Exception:\n\
             \tpass\n"
        ),
        None,
        Some(&locals),
    )?;
    let result = locals.get_item("rules")?.unwrap();
    let list = result.downcast::<PyList>()?;
    Ok(list
        .iter()
        .map(|item| item.into_pyobject(py).unwrap().into())
        .collect())
}

/// Convert a Rust `ModelInfo` to a Python `smelt_sdk.types.ModelInfo` instance.
fn model_info_to_py<'py>(py: Python<'py>, model: &ModelInfo) -> PyResult<Bound<'py, PyAny>> {
    let types = py.import("smelt_sdk.types")?;

    let inc_config = match &model.incremental_config {
        Some(cfg) => {
            let ic = types.getattr("IncrementalConfig")?;
            let granularity_str = match &cfg.granularity {
                crate::types::Granularity::Hour => "hour".to_string(),
                crate::types::Granularity::Day => "day".to_string(),
                crate::types::Granularity::Week { week_start } => {
                    format!(
                        "week:{}",
                        match week_start {
                            crate::types::Weekday::Monday => "monday",
                            crate::types::Weekday::Tuesday => "tuesday",
                            crate::types::Weekday::Wednesday => "wednesday",
                            crate::types::Weekday::Thursday => "thursday",
                            crate::types::Weekday::Friday => "friday",
                            crate::types::Weekday::Saturday => "saturday",
                            crate::types::Weekday::Sunday => "sunday",
                        }
                    )
                }
                crate::types::Granularity::Month => "month".to_string(),
            };
            Some(ic.call1((
                cfg.partition_column.as_str(),
                cfg.event_time_column.as_str(),
                granularity_str,
            ))?)
        }
        None => None,
    };

    let refs_list = PyList::new(py, &model.refs)?;
    let mi_cls = types.getattr("ModelInfo")?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("name", &model.name)?;
    kwargs.set_item("sql", &model.sql)?;
    kwargs.set_item("refs", refs_list)?;
    kwargs.set_item("incremental_config", inc_config)?;
    mi_cls.call((), Some(&kwargs))
}

/// Convert a Rust `SelectAnalysis` to a Python `smelt_sdk.types.SelectAnalysis`.
fn analysis_to_py<'py>(py: Python<'py>, analysis: &SelectAnalysis) -> PyResult<Bound<'py, PyAny>> {
    let types = py.import("smelt_sdk.types")?;

    let items = PyList::empty(py);
    for item in &analysis.items {
        let py_item = match item {
            SelectItemKind::CountDistinct { argument, alias } => {
                let cls = types.getattr("CountDistinct")?;
                cls.call1((argument.as_str(), alias.as_str()))?
            }
            SelectItemKind::OtherAggregate { text, alias } => {
                let cls = types.getattr("OtherAggregate")?;
                cls.call1((text.as_str(), alias.as_str()))?
            }
            SelectItemKind::GroupByKey { text, alias } => {
                let cls = types.getattr("GroupByKey")?;
                cls.call1((text.as_str(), alias.as_str()))?
            }
        };
        items.append(py_item)?;
    }

    let group_by = PyList::new(py, &analysis.group_by_exprs)?;

    let cls = types.getattr("SelectAnalysis")?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("items", items)?;
    kwargs.set_item("from_text", &analysis.from_text)?;
    kwargs.set_item("where_text", &analysis.where_text)?;
    kwargs.set_item("group_by_exprs", group_by)?;
    kwargs.set_item(
        "has_cube_split_annotation",
        analysis.has_cube_split_annotation,
    )?;
    cls.call((), Some(&kwargs))
}

/// Convert a Python transformation object back to a Rust `Transformation`.
fn py_to_transformation(
    py: Python<'_>,
    obj: &Bound<'_, PyAny>,
) -> PyResult<Option<Transformation>> {
    let types = py.import("smelt_sdk.types")?;
    let replace_cls = types.getattr("ReplaceWithPlan")?;
    let set_inc_cls = types.getattr("SetIncremental")?;

    if obj.is_instance(&replace_cls)? {
        let model: String = obj.getattr("model")?.extract()?;
        let py_steps: Vec<Bound<'_, PyAny>> = obj.getattr("steps")?.extract()?;

        let create_cls = types.getattr("CreateTemp")?;
        let append_cls = types.getattr("AppendToTemp")?;
        let final_cls = types.getattr("FinalQuery")?;
        let drop_cls = types.getattr("DropTemp")?;

        let mut steps = Vec::new();
        for step in &py_steps {
            if step.is_instance(&create_cls)? {
                steps.push(ExecutionStep::CreateTemp {
                    name: step.getattr("name")?.extract()?,
                    sql: step.getattr("sql")?.extract()?,
                });
            } else if step.is_instance(&append_cls)? {
                steps.push(ExecutionStep::AppendToTemp {
                    name: step.getattr("name")?.extract()?,
                    sql: step.getattr("sql")?.extract()?,
                });
            } else if step.is_instance(&final_cls)? {
                steps.push(ExecutionStep::FinalQuery {
                    sql: step.getattr("sql")?.extract()?,
                });
            } else if step.is_instance(&drop_cls)? {
                steps.push(ExecutionStep::DropTemp {
                    name: step.getattr("name")?.extract()?,
                });
            }
        }
        Ok(Some(Transformation::ReplaceWithPlan { model, steps }))
    } else if obj.is_instance(&set_inc_cls)? {
        Ok(Some(Transformation::SetIncremental {
            model: obj.getattr("model")?.extract()?,
            event_time_column: obj.getattr("event_time_column")?.extract()?,
            partition_column: obj.getattr("partition_column")?.extract()?,
        }))
    } else {
        Ok(None)
    }
}

/// A discovered Python rule result.
struct PythonRuleResult {
    _rule_name: String,
    transformation: Transformation,
}

/// Run all discovered Python optimizer rules against the given models.
///
/// For each model, pre-computes `SelectAnalysis` in Rust and passes it
/// to Python rules as a dataclass. Returns transformations and errors.
pub fn run_python_rules(models: &[&ModelInfo]) -> (Vec<Transformation>, Vec<String>) {
    let mut transformations = Vec::new();
    let mut errors = Vec::new();

    let result: Result<Vec<PythonRuleResult>, String> = Python::with_gil(|py| {
        let rules = discover_python_rules(py)
            .map_err(|e| format!("Failed to discover Python rules: {}", e))?;

        if rules.is_empty() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();

        for model in models {
            let analysis = analyze_select(&model.sql);

            let py_model = model_info_to_py(py, model).map_err(|e| {
                format!("Failed to convert model '{}' to Python: {}", model.name, e)
            })?;

            let py_analysis = match &analysis {
                Some(a) => {
                    let obj = analysis_to_py(py, a).map_err(|e| {
                        format!(
                            "Failed to convert analysis for model '{}' to Python: {}",
                            model.name, e
                        )
                    })?;
                    Some(obj)
                }
                None => None,
            };

            for rule in &rules {
                let rule_bound = rule.bind(py);
                let rule_name: String = match rule_bound.call_method0("name") {
                    Ok(n) => match n.extract() {
                        Ok(s) => s,
                        Err(e) => {
                            return Err(format!("Python rule name() failed: {}", e));
                        }
                    },
                    Err(e) => {
                        return Err(format!("Python rule name() failed: {}", e));
                    }
                };

                // Call rewrite directly (skip detect for simplicity — rules
                // that don't match return None from rewrite).
                let py_analysis_arg: PyObject = match &py_analysis {
                    Some(a) => a.clone().into(),
                    None => py.None(),
                };

                let rewrite_result =
                    rule_bound.call_method1("rewrite", (&py_model, py_analysis_arg));

                match rewrite_result {
                    Ok(result) => {
                        if !result.is_none() {
                            match py_to_transformation(py, &result) {
                                Ok(Some(t)) => {
                                    results.push(PythonRuleResult {
                                        _rule_name: rule_name.clone(),
                                        transformation: t,
                                    });
                                }
                                Ok(None) => {}
                                Err(e) => {
                                    return Err(format!(
                                        "Python rule '{}' returned invalid transformation for model '{}': {}",
                                        rule_name, model.name, e
                                    ));
                                }
                            }
                        }
                    }
                    Err(e) => {
                        // Get full Python traceback
                        let tb = e
                            .traceback(py)
                            .map(|tb| {
                                tb.format()
                                    .unwrap_or_else(|_| "  <traceback unavailable>".to_string())
                            })
                            .unwrap_or_default();
                        return Err(format!(
                            "Python rule '{}' raised exception for model '{}':\n{}{}\n",
                            rule_name, model.name, tb, e
                        ));
                    }
                }
            }
        }

        Ok(results)
    });

    match result {
        Ok(results) => {
            for r in results {
                transformations.push(r.transformation);
            }
        }
        Err(e) => {
            errors.push(e);
        }
    }

    (transformations, errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::IncrementalConfig;

    #[test]
    fn test_discover_rules_without_packages() {
        // With no packages installed, should return empty
        Python::with_gil(|py| {
            let rules = discover_python_rules(py).unwrap();
            // May or may not find rules depending on environment,
            // but should not error.
            let _ = rules;
        });
    }

    #[test]
    fn test_model_info_roundtrip() {
        Python::with_gil(|py| {
            // Ensure smelt_sdk is importable — skip test if not installed
            if py.import("smelt_sdk").is_err() {
                return;
            }

            let model = ModelInfo {
                name: "test_model".to_string(),
                sql: "SELECT 1".to_string(),
                refs: vec!["other".to_string()],
                incremental_config: Some(IncrementalConfig {
                    partition_column: "dt".to_string(),
                    event_time_column: "event_time".to_string(),
                    granularity: crate::types::Granularity::Day,
                    safety_overrides: crate::types::IncrementalSafetyOverrides::default(),
                }),
            };

            let py_model = model_info_to_py(py, &model).unwrap();
            let name: String = py_model.getattr("name").unwrap().extract().unwrap();
            assert_eq!(name, "test_model");

            let refs: Vec<String> = py_model.getattr("refs").unwrap().extract().unwrap();
            assert_eq!(refs, vec!["other"]);
        });
    }

    #[test]
    fn test_run_python_rules_empty() {
        let models: Vec<&ModelInfo> = Vec::new();
        let (transformations, errors) = run_python_rules(&models);
        assert!(transformations.is_empty());
        assert!(errors.is_empty());
    }
}
