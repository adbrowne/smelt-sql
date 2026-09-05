//! Pure rendering of a [`DiffReport`] (`docs/specs/property_diff.md`
//! §Surface "Output forms"). This is the ONLY renderer of the text and
//! Markdown forms: the CLI's `--diff` path and the editor's
//! lens/diagnostic surfaces all read `report.models` and its per-change
//! fields through the functions here rather than re-deriving ordering or
//! message text
//! (`docs/outcomes/20260905-property-diff/phases/05-plan.md` D5, ruling
//! R4). No function in this module sorts `models` — `diff_profiles`
//! already produced the dependency-then-name order §Surface "Text"
//! requires.
//!
//! [`glyph`], [`dimension_str`], [`json_display`], and [`cause_str`] are
//! the shared field-level rendering primitives every form calls
//! (`docs/specs/property_diff.md` §Constraints item 5, "Surface parity";
//! `docs/outcomes/20260905-property-diff/phases/06-plan.md` D6.2):
//! [`markdown_report`] is not literally [`change_line`]'s string, but
//! every value inside it is produced by these same functions, never
//! re-spelled or reformatted for the Markdown table.

use std::fmt::Write as _;

use serde_json::Value;

use super::diff::{Cause, CauseKind, Change, DiffReport, Dimension, Direction, ModelDiff};

/// The HTML comment marker every rendered form's Markdown body ends with,
/// so a CI workflow can find and update its previous comment instead of
/// stacking a new one on every push
/// (`docs/specs/property_diff.md` §Surface "Markdown", §"Pull-request
/// comment"). `docs-site/docs/guide/ci.md` and
/// `.github/workflows/property-diff.yml` must both quote this literal
/// byte-for-byte — asserted by
/// `crates/smelt-cli/tests/property_diff_ci_docs.rs`.
pub const MARKER: &str = "<!-- smelt-property-diff -->";

/// The maximum number of shifted models `markdown_report` renders in full
/// before falling back to a name-only tail block. GitHub rejects an issue
/// comment body over 65,536 characters
/// (`docs/specs/property_diff.md` §Surface "Markdown", Δ5); capping the
/// number of fully-rendered blocks keeps the body well under that limit
/// regardless of how large the diff is.
const MARKDOWN_MAX_FULL_MODELS: usize = 50;

/// The direction glyph (`docs/specs/property_diff.md` §Surface "Text"):
/// `▼` downgrade, `▲` upgrade, `●` neutral. Public: a shared rendering
/// primitive every form (text, Markdown, and the editor) calls rather than
/// re-deriving its own glyph (§Constraints item 5, "Surface parity").
pub fn glyph(direction: Direction) -> char {
    match direction {
        Direction::Downgrade => '▼',
        Direction::Upgrade => '▲',
        Direction::Neutral => '●',
    }
}

/// The JSON `dimension` string for a [`Dimension`], read off its own
/// `Serialize` impl rather than re-listing the enum's spelling here — the
/// two can never drift (`docs/specs/property_diff.md` §Constraints item 3,
/// "Direction totality"). Public: shared by every rendered form.
pub fn dimension_str(dimension: Dimension) -> String {
    serde_json::to_value(dimension)
        .ok()
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "unknown".to_string())
}

/// Render a JSON `old`/`new` value the way the text form displays it: a
/// bare string prints without quotes, `None` prints `null`, everything
/// else prints as compact JSON. Public: shared by every rendered form.
pub fn json_display(v: &Option<Value>) -> String {
    match v {
        None => "null".to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// One change line: glyph, dimension, subject, `old → new`
/// (`docs/specs/property_diff.md` §Surface "Text").
pub fn change_line(change: &Change) -> String {
    // A whole-model dimension (`grain`, `maintenance_lost`, …) carries an
    // empty `subject` — render `dimension:` directly rather than
    // `dimension :` with a stray double space (fix round 1, Q7).
    if change.subject.is_empty() {
        format!(
            "{} {}: {} → {}",
            glyph(change.direction),
            dimension_str(change.dimension),
            json_display(&change.old),
            json_display(&change.new),
        )
    } else {
        format!(
            "{} {} {}: {} → {}",
            glyph(change.direction),
            dimension_str(change.dimension),
            change.subject,
            json_display(&change.old),
            json_display(&change.new),
        )
    }
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
/// Public: shared by every rendered form.
pub fn cause_str(cause: &Cause) -> String {
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

/// One shifted model's `<details>` block for the Markdown form: a
/// `<summary>` naming the model, its cause, and its per-model change
/// counts, open by default (`<details open>`) when the model holds at
/// least one downgrade (`docs/specs/property_diff.md` §Surface "Markdown",
/// D6.3 — decided from the model's OWN changes, never from
/// `report.summary`, which is a whole-report count and would open every
/// block whenever any model in the report downgraded). The table columns
/// are the JSON `changes` entry fields, and every cell value is produced
/// by the same primitives `text_report` uses (`dimension_str`,
/// `json_display`, `cause_str`) — never re-derived here.
fn markdown_model_block(model: &ModelDiff) -> String {
    let open = model
        .changes
        .iter()
        .any(|c| c.direction == Direction::Downgrade);
    let downgrades = model
        .changes
        .iter()
        .filter(|c| c.direction == Direction::Downgrade)
        .count();
    let upgrades = model
        .changes
        .iter()
        .filter(|c| c.direction == Direction::Upgrade)
        .count();
    let neutral = model
        .changes
        .iter()
        .filter(|c| c.direction == Direction::Neutral)
        .count();

    let mut out = String::new();
    let _ = writeln!(
        out,
        "<details{}>\n<summary>{} — {} — {} downgrades, {} upgrades, {} neutral</summary>\n",
        if open { " open" } else { "" },
        model.model,
        cause_str(&model.cause),
        downgrades,
        upgrades,
        neutral,
    );
    let _ = writeln!(
        out,
        "| dimension | subject | direction | old | new | reason |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|");
    for change in &model.changes {
        let direction_str = match change.direction {
            Direction::Downgrade => "downgrade",
            Direction::Upgrade => "upgrade",
            Direction::Neutral => "neutral",
        };
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} |",
            dimension_str(change.dimension),
            change.subject,
            direction_str,
            json_display(&change.old),
            json_display(&change.new),
            change.reason.as_deref().unwrap_or(""),
        );
    }
    let _ = writeln!(out, "\n</details>\n");
    out
}

/// The Markdown comment body (`docs/specs/property_diff.md` §Surface
/// "Markdown"): a one-line heading with the baseline and summary counts,
/// one `<details>` block per shifted model, and a trailing [`MARKER`] a CI
/// workflow uses to find and update its previous comment. Reuses
/// `report.models`'s own order — already dependency-then-name sorted by
/// `diff_profiles` — exactly as `text_report` does; never re-sorted here.
///
/// The marker is emitted even when nothing shifted (Δ4): without it, a
/// workflow that posted a downgrade comment could never find and clear it
/// once the regression is fixed, leaving a stale warning about code that
/// no longer exists standing on the PR.
///
/// Bounded per Δ5: at most [`MARKDOWN_MAX_FULL_MODELS`] models render in
/// full; any remainder is named, not rendered, inside one final
/// `<details>` block, so the body cannot exceed GitHub's comment size
/// limit regardless of how large the diff is. The cap affects only this
/// rendered body — `report.summary` and `--fail-on` still see every
/// shifted model.
pub fn markdown_report(report: &DiffReport) -> String {
    if report.models.is_empty() {
        return format!(
            "property diff vs {}: no models shifted\n\n{MARKER}\n",
            report.baseline.r#ref
        );
    }

    let mut out = String::new();
    let _ = writeln!(
        out,
        "### property diff vs {} = {} ({} file(s) changed, {} model(s) shifted)\n",
        report.baseline.r#ref,
        report.baseline.commit,
        report.edited_files.len(),
        report.summary.shifted_models,
    );
    let _ = writeln!(
        out,
        "{} downgrades, {} upgrades, {} neutral.\n",
        report.summary.downgrades, report.summary.upgrades, report.summary.neutral
    );

    let total = report.models.len();
    let full_count = total.min(MARKDOWN_MAX_FULL_MODELS);
    for model in &report.models[..full_count] {
        out.push_str(&markdown_model_block(model));
    }
    if total > full_count {
        let remaining = total - full_count;
        let _ = writeln!(
            out,
            "<details>\n<summary>… and {remaining} more shifted models</summary>\n"
        );
        for model in &report.models[full_count..] {
            let _ = writeln!(out, "- {}", model.model);
        }
        let _ = writeln!(out, "\n</details>\n");
    }

    let _ = writeln!(out, "{MARKER}");
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

    #[test]
    fn change_line_has_no_double_space_for_an_empty_subject() {
        // Fix round 1, Q7: a whole-model dimension (empty `subject`) must
        // not render "dimension :" with a stray double space.
        let c = Change {
            dimension: Dimension::MaintenanceLost,
            subject: String::new(),
            direction: Direction::Downgrade,
            old: Some(Value::Bool(true)),
            new: Some(Value::Bool(false)),
            reason: None,
            kind: crate::analysis::diff::ChangeKind::MaintenanceLost,
        };
        let line = change_line(&c);
        assert!(
            !line.contains("  "),
            "expected no double space in a subject-less change line: {line:?}"
        );
        assert_eq!(line, "▼ maintenance_lost: true → false");
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

    // --- Markdown form (`docs/outcomes/20260905-property-diff/phases/06-plan.md`) ---

    #[test]
    fn markdown_report_of_an_empty_diff_still_carries_the_marker() {
        // Δ4: the marker and heading are emitted even on an empty diff, so
        // a workflow can find and clear a previously-posted downgrade
        // comment once the regression is fixed. An implementation that
        // early-returns the text form's bare line has no marker at all.
        let body = markdown_report(&empty_report());
        assert!(
            body.contains("no models shifted"),
            "expected the cleared-state line: {body:?}"
        );
        assert!(
            body.trim_end().ends_with(MARKER),
            "marker must be the last line: {body:?}"
        );
    }

    #[test]
    fn a_model_with_a_downgrade_renders_details_open() {
        let downgraded = model_diff(Cause {
            kind: CauseKind::Edited,
            of: vec![],
            reason: None,
        });
        let mut neutral_model = model_diff(Cause {
            kind: CauseKind::Edited,
            of: vec![],
            reason: None,
        });
        neutral_model.model = "bbb.untouched".to_string();
        neutral_model.changes = vec![change(Direction::Neutral)];

        let report = DiffReport {
            baseline: baseline(),
            edited_files: vec![],
            summary: DiffSummary {
                downgrades: 1,
                upgrades: 0,
                neutral: 1,
                shifted_models: 2,
            },
            models: vec![downgraded, neutral_model],
        };
        let body = markdown_report(&report);
        // D6.3: open-state is decided per-model from its own changes, not
        // from `report.summary.downgrades > 0` — that wrong implementation
        // would also open bbb.untouched's block. Exactly one <details open>.
        assert_eq!(
            body.matches("<details open>").count(),
            1,
            "expected exactly one open block: {body}"
        );
        assert!(body.contains("<details>\n<summary>bbb.untouched"));
    }

    #[test]
    fn markdown_values_match_the_text_form() {
        let report = DiffReport {
            baseline: baseline(),
            edited_files: vec![],
            summary: DiffSummary {
                downgrades: 1,
                upgrades: 1,
                neutral: 1,
                shifted_models: 1,
            },
            models: vec![ModelDiff {
                model: "staging.orders".to_string(),
                cause: Cause {
                    kind: CauseKind::Edited,
                    of: vec![],
                    reason: None,
                },
                changes: vec![
                    change(Direction::Downgrade),
                    change(Direction::Upgrade),
                    change(Direction::Neutral),
                ],
            }],
        };
        let text = text_report(&report);
        let markdown = markdown_report(&report);

        for c in &report.models[0].changes {
            assert!(
                markdown.contains(&dimension_str(c.dimension)),
                "dimension_str must appear in markdown"
            );
            assert!(text.contains(&dimension_str(c.dimension)));
            assert!(markdown.contains(&c.subject));
            assert!(text.contains(&c.subject));
            assert!(markdown.contains(&json_display(&c.old)));
            assert!(text.contains(&json_display(&c.old)));
            assert!(markdown.contains(&json_display(&c.new)));
            assert!(text.contains(&json_display(&c.new)));
        }
        let cause = cause_str(&report.models[0].cause);
        assert!(markdown.contains(&cause));
        assert!(text.contains(&cause));
    }

    #[test]
    fn markdown_preserves_diff_profiles_ordering() {
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
        let markdown = markdown_report(&report);
        let staging_pos = markdown.find("staging.orders").unwrap();
        let aaa_pos = markdown.find("aaa.first").unwrap();
        assert!(
            staging_pos < aaa_pos,
            "renderer must preserve report.models order, not re-sort alphabetically"
        );
    }

    #[test]
    fn markdown_table_columns_match_the_json_change_keys() {
        let report = DiffReport {
            baseline: baseline(),
            edited_files: vec![],
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
        let markdown = markdown_report(&report);
        assert!(
            markdown.contains("| dimension | subject | direction | old | new | reason |"),
            "expected exactly the JSON change keys as columns: {markdown}"
        );
    }

    #[test]
    fn markdown_body_of_a_large_diff_stays_under_the_comment_limit() {
        let models: Vec<ModelDiff> = (0..500)
            .map(|i| ModelDiff {
                model: format!("model_{i:04}"),
                cause: Cause {
                    kind: CauseKind::Edited,
                    of: vec![],
                    reason: None,
                },
                changes: vec![change(Direction::Downgrade)],
            })
            .collect();
        let report = DiffReport {
            baseline: baseline(),
            edited_files: vec![],
            summary: DiffSummary {
                downgrades: 500,
                upgrades: 0,
                neutral: 0,
                shifted_models: 500,
            },
            models,
        };
        let body = markdown_report(&report);
        assert!(
            body.len() < 65_536,
            "expected a bounded body, got {} bytes",
            body.len()
        );
        assert!(
            body.contains("and 450 more shifted models"),
            "expected the capped-tail line: last 500 chars = {:?}",
            &body[body.len().saturating_sub(500)..]
        );
        assert!(
            body.trim_end().ends_with(MARKER),
            "marker must still be the last line on a capped body"
        );
    }
}
