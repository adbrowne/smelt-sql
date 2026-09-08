//! Invocation of externally-produced sources' black-box steps on the run
//! path (`docs/specs/sources.md` §"Externally-produced sources (black-box
//! steps)"). A required step runs to completion, sequentially, before any
//! model executes (§Semantics 9) — ordering ahead of every consumer is
//! structural (a single pass before the model loop), not scheduled into
//! `execution_waves`.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use smelt_core::external_step::{resolve_command, ExternalStepInfo, StepRunContext};

/// `ExternalStepNotInvocable` (`docs/specs/sources.md` §Semantics 12): a run
/// reached a step it may not or cannot invoke — a dry run, an environment
/// declining external invocation, an unspawnable `command:`, or a
/// placeholder with no value this run. Reading the produced sources'
/// possibly-stale existing content instead is never the fallback.
#[derive(Debug, Error)]
#[error("ExternalStepNotInvocable: step '{step}' cannot be invoked this run — {reason}")]
pub struct ExternalStepNotInvocableError {
    pub step: String,
    pub reason: String,
}

/// `ExternalStepFailed` (`docs/specs/sources.md` §Semantics 11): the step's
/// `command:` exited non-zero. Every model downstream of the sources it
/// produces is left unbuilt.
#[derive(Debug, Error)]
#[error("ExternalStepFailed: step '{step}' exited with code {exit_code}")]
pub struct ExternalStepFailedError {
    pub step: String,
    pub exit_code: i32,
}

/// Invoke every step in `required_steps`, in the given (sorted) order,
/// before any model executes. Refusal checks run first: a dry run or an
/// environment declining external invocation (`invoke_external_steps ==
/// false`) refuses with `ExternalStepNotInvocable` rather than spawning
/// anything, so a run never proceeds against a reached step's possibly-
/// stale produced sources.
pub(crate) async fn invoke_required_steps(
    required_steps: &[String],
    steps_by_addr: &HashMap<String, ExternalStepInfo>,
    project_dir: &Path,
    ctx: &StepRunContext,
    dry_run: bool,
    invoke_external_steps: bool,
    cancel: &CancellationToken,
) -> Result<()> {
    if required_steps.is_empty() {
        return Ok(());
    }

    if dry_run {
        return Err(ExternalStepNotInvocableError {
            step: required_steps[0].clone(),
            reason: "this is a dry run — `smelt explain` is the non-refusing preview surface \
                     for a step"
                .to_string(),
        }
        .into());
    }
    if !invoke_external_steps {
        return Err(ExternalStepNotInvocableError {
            step: required_steps[0].clone(),
            reason: "this run environment does not invoke external steps".to_string(),
        }
        .into());
    }

    for step_addr in required_steps {
        let Some(step) = steps_by_addr.get(step_addr) else {
            continue;
        };
        let argv = resolve_command(step, ctx).map_err(|e| ExternalStepNotInvocableError {
            step: step_addr.clone(),
            reason: e.to_string(),
        })?;
        let Some((program, args)) = argv.split_first() else {
            continue;
        };

        tracing::info!("Invoking external step '{}': {:?}", step_addr, argv);

        let mut command = tokio::process::Command::new(program);
        command.args(args).current_dir(project_dir);
        let mut child = command.spawn().map_err(|e| ExternalStepNotInvocableError {
            step: step_addr.clone(),
            reason: format!("failed to spawn '{program}': {e}"),
        })?;

        let status = tokio::select! {
            status = child.wait() => status.map_err(|e| ExternalStepNotInvocableError {
                step: step_addr.clone(),
                reason: format!("failed to wait on '{program}': {e}"),
            })?,
            _ = cancel.cancelled() => {
                let _ = child.start_kill();
                anyhow::bail!("Run cancelled while invoking external step '{}'", step_addr);
            }
        };

        if !status.success() {
            return Err(ExternalStepFailedError {
                step: step_addr.clone(),
                exit_code: status.code().unwrap_or(-1),
            }
            .into());
        }

        tracing::info!("External step '{}' completed successfully", step_addr);
    }

    Ok(())
}
