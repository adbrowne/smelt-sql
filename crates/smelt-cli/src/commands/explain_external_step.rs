//! `smelt explain <address>` when `<address>` resolves to an external step
//! rather than a model (phase 6 of
//! `docs/outcomes/20260906-external-dag-steps`).
//!
//! A step is selector-addressable (`docs/specs/model_selection.md`
//! §"Selection methods") through the same `resolve_node_path` resolution a
//! model name uses, but it has no maintenance plan — `--show-sql`,
//! `--period`, and `--technique` are meaningless for it and are rejected as
//! usage errors rather than silently ignored. `explain` never spawns the
//! step's `command:`; this is what lets `docs/specs/sources.md` §Semantics
//! 12 name it the non-refusing preview surface for a step, unlike a dry run.
//!
//! [`dispatch`] resolves the positional argument and returns `Ok(true)` once
//! it has rendered a step's report, `Ok(false)` when the argument resolves
//! to a model (or to nothing at all) so the caller falls through to the
//! existing maintenance-plan path — which keeps owning the "not found"
//! error for an address that is neither.

use anyhow::{Context, Result};
use smelt_cli::argument_resolution::{compute_scope, resolve_argument};
use smelt_cli::explain::ExplainExternalStep;
use smelt_cli::{discover_python_models, find_project_root, init_db, Config, ModelDiscovery};
use smelt_core::external_step::ExternalStepInfo;
use smelt_core::graph::DependencyGraph;
use thiserror::Error;

use crate::ExplainArgs;

/// A maintenance-plan-only flag applied to a step. `docs/specs/cli.md`
/// §"`smelt explain <external step>`" — exit `2`, naming both the step and
/// the flag, rather than the generic "not found" or a silent no-op.
#[derive(Debug, Error)]
#[error("`{flag}` has no meaning for external step '{step}' — a step has no maintenance plan")]
pub struct UsageFlagOnStep {
    step: String,
    flag: &'static str,
}

/// `smelt explain`'s own exit-code classifier: a [`UsageFlagOnStep`] refusal
/// maps to `2` (usage error), same pattern as `commands::list::exit_code_for`.
pub fn exit_code_for(err: &anyhow::Error) -> u8 {
    if err.downcast_ref::<UsageFlagOnStep>().is_some() {
        2
    } else {
        smelt_cli::exit_code_for(err)
    }
}

/// Resolve `name` and, if it names a discovered external step, print that
/// step's report and return `Ok(true)`. Returns `Ok(false)` when `name`
/// resolves to a model or to nothing — the caller owns what happens next.
pub async fn dispatch(args: &ExplainArgs, name: &str, scope: Option<&str>) -> Result<bool> {
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;
    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;

    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let mut models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;
    let python_files = discovery
        .discover_python_files()
        .with_context(|| "Failed to scan for Python models")?;
    if !python_files.is_empty() {
        let python_models = discover_python_models(
            &python_files,
            &models,
            &config,
            &project_dir,
            config.python.as_deref(),
        )
        .with_context(|| "Failed to discover Python models")?;
        models.extend(python_models);
    }

    let db = init_db(&project_dir, &models);
    let ws = smelt_db::Workspace::try_get(&db).expect("workspace not initialized");
    let project = db
        .project_input(&project_dir)
        .expect("project not initialized");

    let cwd = std::env::current_dir().unwrap_or_else(|_| project_dir.clone());
    let active_scope = compute_scope(&project_dir, &cwd, &config.paths, scope);
    let Ok(canonical) = resolve_argument(&db, ws, project, active_scope.as_ref(), name) else {
        return Ok(false);
    };

    let external_steps = smelt_core::discover_external_steps(&project_dir, &config.paths);
    let Some(step) = external_steps
        .iter()
        .find(|s| s.address_segments.join(".") == canonical)
    else {
        return Ok(false);
    };

    if args.show_sql {
        return Err(UsageFlagOnStep {
            step: canonical,
            flag: "--show-sql",
        }
        .into());
    }
    if args.period.is_some() {
        return Err(UsageFlagOnStep {
            step: canonical,
            flag: "--period",
        }
        .into());
    }
    if args.technique.is_some() {
        return Err(UsageFlagOnStep {
            step: canonical,
            flag: "--technique",
        }
        .into());
    }

    let sources = smelt_cli::SourcesConfig::load(&project_dir).ok();
    let mut graph = DependencyGraph::build(models, sources.as_ref())
        .with_context(|| "Failed to build dependency graph")?;
    graph.add_external_steps(&external_steps);
    let consumers = graph.consumers_of_step(&canonical).to_vec();

    render(args, &canonical, step, &consumers)?;
    Ok(true)
}

/// One external step's `--json` object: `kind`/`address` plus the shared
/// [`ExplainExternalStep`] fields, flattened so the shape matches
/// `docs/specs/cli.md` §"`smelt explain <external step>`" exactly.
#[derive(serde::Serialize)]
struct ExplainExternalStepJson<'a> {
    kind: &'static str,
    address: String,
    #[serde(flatten)]
    step: &'a ExplainExternalStep,
}

fn render(
    args: &ExplainArgs,
    canonical: &str,
    step: &ExternalStepInfo,
    consumers: &[String],
) -> Result<()> {
    let rendered = ExplainExternalStep {
        produces: step.produces.clone(),
        command: step.command.clone(),
        cadence: step.cadence.as_ref().map(|c| c.display.clone()),
        description: step.description.clone(),
        consumers: consumers.to_vec(),
    };

    if args.json {
        let json = ExplainExternalStepJson {
            kind: "external_step",
            address: format!("smelt.{canonical}"),
            step: &rendered,
        };
        println!("{}", serde_json::to_string_pretty(&json)?);
        return Ok(());
    }

    println!("External step: smelt.{canonical}");
    if let Some(desc) = &rendered.description {
        println!("  {desc}");
    }
    println!("  produces: {}", rendered.produces.join(", "));
    println!("  command: {}", rendered.command.join(" "));
    if let Some(cadence) = &rendered.cadence {
        println!("  cadence: {cadence}");
    }
    if rendered.consumers.is_empty() {
        println!("  consumers: (none)");
    } else {
        println!("  consumers: {}", rendered.consumers.join(", "));
    }
    println!();
    println!(
        "smelt does not author or parse this step's program — it is invoked and observed by exit code only."
    );

    Ok(())
}
