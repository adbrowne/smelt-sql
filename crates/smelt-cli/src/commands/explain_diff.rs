//! `smelt explain --diff` (`docs/specs/property_diff.md`): resolve a git
//! baseline, derive every model's property profile at both versions, diff
//! them, and render the result as text (default) or JSON.
//!
//! Sequencing (`docs/outcomes/20260905-property-diff/phases/05-plan.md`
//! D2): the working tree loads and derives FIRST, so a broken working tree
//! fails before any scratch directory exists; only then is the baseline
//! resolved and materialised. The [`smelt_core::baseline::BaselineCheckout`]
//! is held until every value derived from it (the baseline's profiles and
//! the edited set) has been read, then dropped — its `Drop` deletes the
//! scratch directory (`docs/specs/property_diff.md` §Constraints item 8).
//!
//! Rendering (text and Markdown) is delegated entirely to
//! `smelt_logical::analysis::diff_render` over the `DiffReport` envelope
//! (`docs/specs/property_diff.md` §Surface "Output forms") — this module
//! assembles the report's inputs and never formats a change itself. Same
//! for the §Semantics rules `apply_failure_reasons` (C2) and
//! `DiffReport::narrow_to` (`--select`, D6): both are single-owned in
//! `smelt_logical::analysis::diff` (fix round 1, Q6) so Phase 6's Markdown
//! renderer and Phase 7's LSP reuse them rather than each reimplementing
//! the same rule.

use anyhow::{Context, Result};
use smelt_cli::argument_resolution::{compute_scope, resolve_selector_args};
use smelt_cli::{find_project_root, init_db, parse_selector, CliError, Config};
use smelt_logical::analysis::diff_render::{markdown_report, text_report};
use smelt_runtime::property_diff::{
    baseline_side, report as diff_report, work_side, PropertyDiffError,
};

use crate::ExplainArgs;

/// Convert a working-tree pipeline error to `anyhow`, preserving the exit
/// code contract: a bare working tree never produces a `Baseline` error
/// (only `baseline_side` resolves git), so every variant here gets a
/// helpful context message and exit code `1`.
fn property_diff_err_for_working_tree(err: PropertyDiffError) -> anyhow::Error {
    anyhow::Error::new(err).context("Failed to derive property profiles for the working tree")
}

/// Convert a baseline-side pipeline error to `anyhow` WITHOUT wrapping a
/// [`smelt_core::baseline::BaselineError`] in `.context(...)`:
/// `smelt_cli::exit_code_for` downcasts the top-level wrapped error type to
/// decide usage-error exit code `2`
/// (`docs/specs/property_diff.md` §Surface, `PropertyDiffBaselineUnavailable`),
/// and `anyhow`'s context-aware downcast only looks at the type immediately
/// wrapped by `.context()`, not its own transitive source chain — so a
/// `Baseline` variant must convert `?`-style (no context) to stay
/// downcastable, exactly as it did before this pipeline was extracted into
/// `smelt-runtime`.
fn property_diff_err_for_baseline(err: PropertyDiffError) -> anyhow::Error {
    match err {
        PropertyDiffError::Baseline(e) => e.into(),
        other => {
            anyhow::Error::new(other).context("Failed to derive property profiles for the baseline")
        }
    }
}

/// Run `smelt explain --diff`. `explicit_ref` is `args.diff`'s inner value
/// (`None` ⇒ default merge-base baseline).
///
/// Sequencing (D2, preserved by the shared pipeline): `work_side` loads and
/// derives the working tree FIRST, so a broken working tree fails before
/// any scratch directory exists; only then does `baseline_side` resolve and
/// materialise the baseline. The pipeline itself is single-owned in
/// `smelt_runtime::property_diff` so the editor (Phase 7) reuses it rather
/// than re-deriving the comparison (`docs/specs/property_diff.md`
/// §Constraints item 5).
pub async fn explain_diff(args: &ExplainArgs, explicit_ref: Option<&str>) -> Result<()> {
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    let work =
        work_side(&project_dir, &Default::default()).map_err(property_diff_err_for_working_tree)?;
    let base = baseline_side(&project_dir, explicit_ref).map_err(property_diff_err_for_baseline)?;

    let mut report = diff_report(&work, &base);

    // D6: --select narrows the REPORTED set only (Δ2) — the compared set
    // (everything derived above) is untouched, so attribution stays
    // correct.
    if !args.select.is_empty() {
        let config =
            Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;
        let db = init_db(&project_dir, &work.loaded.sql_files);
        let ws = smelt_db::Workspace::try_get(&db)
            .ok_or_else(|| anyhow::anyhow!("workspace not initialized"))?;
        let project = db
            .project_input(&project_dir)
            .ok_or_else(|| anyhow::anyhow!("project not initialized"))?;
        let cwd = std::env::current_dir().unwrap_or_else(|_| project_dir.clone());
        let active_scope = compute_scope(&project_dir, &cwd, &config.paths, None);
        let resolved_select =
            resolve_selector_args(&db, ws, project, active_scope.as_ref(), &args.select)
                .map_err(|e| anyhow::anyhow!("{}", e))?;
        let selectors: Vec<_> = resolved_select
            .iter()
            .map(|s| parse_selector(s).with_context(|| format!("Invalid selector '{}'", s)))
            .collect::<Result<_, _>>()?;
        let selected = work
            .graph
            .select_models(&selectors, &config)
            .with_context(|| "Failed to select models")?;
        report.narrow_to(&selected.into_iter().collect());
    }

    // D6.4: the body is always printed here, BEFORE the `--fail-on` early
    // return below. A `--markdown` body printed after that return would be
    // empty exactly when it matters most — a PR carrying a downgrade,
    // where `--fail-on` exits non-zero
    // (`docs/outcomes/20260905-property-diff/phases/06-plan.md` R6).
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).with_context(|| "Failed to serialize diff")?
        );
    } else if args.markdown {
        print!("{}", markdown_report(&report));
    } else {
        print!("{}", text_report(&report));
    }

    if let Some(fail_on) = &args.fail_on {
        let should_fail = match fail_on.as_str() {
            "downgrade" => report.summary.downgrades > 0,
            "any" => report.summary.shifted_models > 0,
            _ => false,
        };
        if should_fail {
            return Err(CliError::DetectedFailure(format!(
                "property diff: --fail-on {fail_on} condition held ({} downgrades, {} shifted \
                 models)",
                report.summary.downgrades, report.summary.shifted_models
            ))
            .into());
        }
    }

    Ok(())
}
