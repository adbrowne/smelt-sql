//! Pure text rendering of a [`DiffReport`] (`docs/specs/property_diff.md`
//! §Surface "Output forms" — "Text"). This is the ONLY renderer of the
//! text form: the CLI's `--diff` path, and later the Markdown form and the
//! editor's lens/diagnostic surfaces, all read `report.models` and its
//! per-change fields through the functions here rather than re-deriving
//! ordering or message text
//! (`docs/outcomes/20260905-property-diff/phases/05-plan.md` D5, ruling
//! R4). No function in this module sorts `models` — `diff_profiles`
//! already produced the dependency-then-name order §Surface "Text"
//! requires.

use std::fmt::Write as _;

use serde_json::Value;

use super::diff::{Cause, CauseKind, Change, DiffReport, Dimension, Direction, ModelDiff};

/// The direction glyph (`docs/specs/property_diff.md` §Surface "Text"):
/// `▼` downgrade, `▲` upgrade, `●` neutral.
fn glyph(direction: Direction) -> char {
    match direction {
        Direction::Downgrade => '▼',
        Direction::Upgrade => '▲',
        Direction::Neutral => '●',
    }
}

/// The JSON `dimension` string for a [`Dimension`], read off its own
/// `Serialize` impl rather than re-listing the enum's spelling here — the
/// two can never drift (`docs/specs/property_diff.md` §Constraints item 3,
/// "Direction totality").
fn dimension_str(dimension: Dimension) -> String {
    serde_json::to_value(dimension)
        .ok()
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "unknown".to_string())
}

/// Render a JSON `old`/`new` value the way the text form displays it: a
/// bare string prints without quotes, `None` prints `null`, everything
/// else prints as compact JSON.
fn json_display(v: &Option<Value>) -> String {
    match v {
        None => "null".to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// One change line: glyph, dimension, subject, `old → new`
/// (`docs/specs/property_diff.md` §Surface "Text").
pub fn change_line(change: &Change) -> String {
    format!(
        "{} {} {}: {} → {}",
        glyph(change.direction),
        dimension_str(change.dimension),
        change.subject,
        json_display(&change.old),
        json_display(&change.new),
    )
}

/// The change's one-line reason, when the derivation exposed one
/// (`docs/specs/property_diff.md` §Surface "Text"). `None` when the
/// dimension carries no reason.
pub fn reason_line(change: &Change) -> Option<String> {
    change.reason.as_ref().map(|r| format!("reason: {r}"))
}

/// The model header's cause annotation: `(edited)`, `(added)`, `(removed)`,
/// or `(downstream of <model>[, <model>…])` — or, when no edited ancestor
/// was found (`of: []`), the model-level reason in its place
/// (`docs/specs/property_diff.md` §Surface "Text", §"Attribution").
fn cause_str(cause: &Cause) -> String {
    match cause.kind {
        CauseKind::Edited => "edited".to_string(),
        CauseKind::Added => match &cause.reason {
            Some(r) => format!("added: {r}"),
            None => "added".to_string(),
        },
        CauseKind::Removed => match &cause.reason {
            Some(r) => format!("removed: {r}"),
            None => "removed".to_string(),
        },
        CauseKind::Downstream => {
            if cause.of.is_empty() {
                match &cause.reason {
                    Some(r) => format!("downstream: {r}"),
                    None => "downstream".to_string(),
                }
            } else {
                format!("downstream of {}", cause.of.join(", "))
            }
        }
    }
}

/// One shifted model's block: header line with its cause, then one line
/// per change (plus its reason line, when present)
/// (`docs/specs/property_diff.md` §Surface "Text").
pub fn model_block(model: &ModelDiff) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "  {}  ({})", model.model, cause_str(&model.cause));
    for change in &model.changes {
        let _ = writeln!(out, "    {}", change_line(change));
        if let Some(reason) = reason_line(change) {
            let _ = writeln!(out, "        {reason}");
        }
    }
    out
}

/// The whole text report (`docs/specs/property_diff.md` §Surface "Text").
/// Blocks render in `report.models`'s own order — already
/// dependency-then-name sorted by `diff_profiles`, never re-sorted here.
/// When nothing shifted, the whole output is the single line
/// `property diff vs <ref>: no models shifted`.
pub fn text_report(report: &DiffReport) -> String {
    if report.models.is_empty() {
        return format!(
            "property diff vs {}: no models shifted\n",
            report.baseline.r#ref
        );
    }
    let mut out = String::new();
    let _ = writeln!(
        out,
        "property diff vs {} = {} ({} file(s) changed, {} model(s) shifted)",
        report.baseline.r#ref,
        report.baseline.commit,
        report.edited_files.len(),
        report.summary.shifted_models,
    );
    let _ = writeln!(out);
    for model in &report.models {
        out.push_str(&model_block(model));
        let _ = writeln!(out);
    }
    let _ = writeln!(
        out,
        "{} downgrades, {} upgrades, {} neutral.",
        report.summary.downgrades, report.summary.upgrades, report.summary.neutral
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::diff::{BaselineInfo, DiffSummary};

    fn baseline() -> BaselineInfo {
        BaselineInfo {
            r#ref: "main".to_string(),
            commit: "abc1234".to_string(),
            resolved_as: "merge_base".to_string(),
        }
    }

    fn empty_report() -> DiffReport {
        DiffReport {
            baseline: baseline(),
            edited_files: vec![],
            summary: DiffSummary::default(),
            models: vec![],
        }
    }

    #[test]
    fn text_report_of_an_empty_diff_is_one_line() {
        assert_eq!(
            text_report(&empty_report()),
            "property diff vs main: no models shifted\n"
        );
    }

    fn change(direction: Direction) -> Change {
        Change {
            dimension: Dimension::CellTechnique,
            subject: "revenue@orders".to_string(),
            direction,
            old: Some(Value::String("KeyedFold".to_string())),
            new: Some(Value::String("DeleteInsert".to_string())),
            reason: None,
            kind: crate::analysis::diff::ChangeKind::Grain {
                subject: "revenue@orders".to_string(),
                old: crate::analysis::walk::Grain::unkeyed(),
                new: crate::analysis::walk::Grain::unkeyed(),
            },
        }
    }

    #[test]
    fn change_line_uses_the_specified_glyphs() {
        assert!(change_line(&change(Direction::Downgrade)).starts_with('▼'));
        assert!(change_line(&change(Direction::Upgrade)).starts_with('▲'));
        assert!(change_line(&change(Direction::Neutral)).starts_with('●'));
    }

    fn model_diff(cause: Cause) -> ModelDiff {
        ModelDiff {
            model: "staging.orders".to_string(),
            cause,
            changes: vec![change(Direction::Downgrade)],
        }
    }

    #[test]
    fn model_block_headers_render_each_cause_kind() {
        assert!(model_block(&model_diff(Cause {
            kind: CauseKind::Edited,
            of: vec![],
            reason: None
        }))
        .contains("(edited)"));
        assert!(model_block(&model_diff(Cause {
            kind: CauseKind::Added,
            of: vec![],
            reason: None
        }))
        .contains("(added)"));
        assert!(model_block(&model_diff(Cause {
            kind: CauseKind::Removed,
            of: vec![],
            reason: None
        }))
        .contains("(removed)"));
        assert!(model_block(&model_diff(Cause {
            kind: CauseKind::Downstream,
            of: vec!["staging.a".to_string(), "staging.b".to_string()],
            reason: None
        }))
        .contains("(downstream of staging.a, staging.b)"));
        let block = model_block(&model_diff(Cause {
            kind: CauseKind::Downstream,
            of: vec![],
            reason: Some("project configuration changed".to_string()),
        }));
        assert!(block.contains("project configuration changed"));
    }

    #[test]
    fn text_report_preserves_diff_profiles_ordering() {
        let report = DiffReport {
            baseline: baseline(),
            edited_files: vec!["models/z.sql".to_string()],
            summary: DiffSummary {
                downgrades: 2,
                upgrades: 0,
                neutral: 0,
                shifted_models: 2,
            },
            models: vec![
                model_diff(Cause {
                    kind: CauseKind::Edited,
                    of: vec![],
                    reason: None,
                }),
                ModelDiff {
                    model: "aaa.first".to_string(),
                    cause: Cause {
                        kind: CauseKind::Edited,
                        of: vec![],
                        reason: None,
                    },
                    changes: vec![change(Direction::Downgrade)],
                },
            ],
        };
        let text = text_report(&report);
        let staging_pos = text.find("staging.orders").unwrap();
        let aaa_pos = text.find("aaa.first").unwrap();
        assert!(
            staging_pos < aaa_pos,
            "renderer must preserve report.models order, not re-sort alphabetically"
        );
    }

    #[test]
    fn report_json_matches_the_spec_schema_keys() {
        let report = DiffReport {
            baseline: baseline(),
            edited_files: vec!["models/a.sql".to_string()],
            summary: DiffSummary {
                downgrades: 1,
                upgrades: 0,
                neutral: 0,
                shifted_models: 1,
            },
            models: vec![model_diff(Cause {
                kind: CauseKind::Edited,
                of: vec![],
                reason: None,
            })],
        };
        let v = serde_json::to_value(&report).unwrap();
        let obj = v.as_object().unwrap();
        let mut keys: Vec<&str> = obj.keys().map(|s| s.as_str()).collect();
        keys.sort();
        assert_eq!(keys, vec!["baseline", "edited_files", "models", "summary"]);
        let baseline_obj = v["baseline"].as_object().unwrap();
        let mut bkeys: Vec<&str> = baseline_obj.keys().map(|s| s.as_str()).collect();
        bkeys.sort();
        assert_eq!(bkeys, vec!["commit", "ref", "resolved_as"]);

        // A cause with no reason omits the key (Δ1).
        let cause_json = &v["models"][0]["cause"];
        assert!(cause_json.as_object().unwrap().get("reason").is_none());
    }
}
