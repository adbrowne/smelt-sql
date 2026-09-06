//! Editor integration for the property diff
//! (`docs/specs/property_diff.md` §Surface "Editor";
//! `docs/outcomes/20260905-property-diff/phases/07-plan.md`).
//!
//! Everything here is pure over `(DiffReport, parsed AST, model->path map)`
//! so it is directly unit-testable without a running server — `backend.rs`
//! is the only place that touches the network/Salsa/filesystem to produce
//! those inputs and cache the result.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rowan::TextRange;
use smelt_db::{Diagnostic as DbDiagnostic, DiagnosticCode, DiagnosticSeverity};
use smelt_logical::analysis::diff::{DiffReport, ModelDiff};
use smelt_logical::analysis::diff_render::lens_title;
use smelt_logical::analysis::diff_stories::{Severity, StoryKind};
use smelt_parser::ast::File as AstFile;
use smelt_runtime::property_diff::{
    baseline_side as derive_baseline_side, report as derive_report, work_side, BaselineSide,
};

/// One project's cached property-diff state
/// (`docs/outcomes/20260905-property-diff/phases/07-plan.md` D2/D3). Keyed
/// on `(project_root, commit)` at the `Backend` level (one entry per
/// project — project isolation); this struct is everything a single
/// project needs to answer `code_lens`/`publish_diagnostics` without
/// re-deriving.
#[derive(Default)]
pub struct ProjectDiffState {
    /// `(commit, ...)` — re-resolution (comparing the freshly resolved
    /// commit to this one), not the `.git` watch, is what decides whether
    /// the cached baseline side is still valid (D2: several clients never
    /// report `.git` changes).
    pub baseline_commit: Option<String>,
    pub report: Option<std::sync::Arc<DiffReport>>,
    /// Model file path -> lens title, for `code_lens`.
    pub lenses: HashMap<PathBuf, String>,
    /// Model file path -> the `PropertyDowngrade` diagnostics to append in
    /// `publish_diagnostics` (D6: a second `publish_diagnostics` call would
    /// clobber, or be clobbered by, the Salsa set — these are merged into
    /// the existing publish, never sent on their own channel). Kept as
    /// `smelt_db::Diagnostic` (raw `rowan::TextRange`, not `lsp_types`)
    /// so the conversion to `(line, column)` still happens exactly once,
    /// through `Backend::to_lsp_diagnostic`'s `BoundaryConverter`
    /// (the diagnostic-range-encoding invariant).
    pub diagnostics: HashMap<PathBuf, Vec<DbDiagnostic>>,
    /// Set when the workspace is not a git work tree, or the baseline
    /// cannot be resolved (`docs/specs/property_diff.md` §Surface
    /// "Editor"): no lens, no diagnostic, logged at `info` only
    /// (fail-loud discipline is satisfied by the log, not a diagnostic —
    /// an un-versioned workspace is not an error).
    pub silent_reason: Option<String>,
    /// Coalesces concurrent refresh triggers
    /// (`docs/outcomes/20260905-property-diff/phases/07-plan.md` D7).
    pub running: bool,
    /// Set when a refresh is requested WHILE `running` is already true
    /// (risk R3: a burst of events must not each pay the full pipeline
    /// cost). `Backend::refresh_property_diff` schedules exactly one
    /// trailing re-run when the in-flight one finishes and this is set,
    /// then clears it — never more than one extra run per burst, however
    /// many triggers landed mid-flight.
    pub pending: bool,
    /// The cached baseline side, reused across refreshes while
    /// `baseline_commit` still matches the freshly re-resolved commit (D2).
    pub cached_baseline: Option<Arc<BaselineSide>>,
}

/// Anchor a story at the narrowest range its `subject` supports
/// (`docs/specs/property_diff.md` §Surface "Editor", "A story's anchor is
/// only as narrow as its `subject` supports"):
///
/// 1. A column-subject kind (`column_semantics`) anchors on the matching
///    `SELECT`-list item's alias, or the whole item if unaliased.
/// 2. A source-subject kind (`reads`, `dependency`) anchors on the matching
///    `FROM`/`JOIN` item.
/// 3. Everything else (a cell subject, a refusal, or a whole-model story
///    with no subject at all — including `schema`, which is never surfaced
///    as a diagnostic in the first place) has no narrower anchor — it is
///    not derivable, and this function does not pretend otherwise (ruling
///    R2) — and anchors at the model's first SQL token, or an empty range
///    at offset 0 if the file has no SQL at all.
pub fn anchor_for(kind: StoryKind, subject: &str, ast: &AstFile) -> TextRange {
    match kind {
        StoryKind::ColumnSemantics => {
            if let Some(range) = anchor_column(subject, ast) {
                return range;
            }
            first_sql_token_range(ast)
        }
        StoryKind::Reads | StoryKind::Dependency => {
            if let Some(range) = anchor_source(subject, ast) {
                return range;
            }
            first_sql_token_range(ast)
        }
        _ => first_sql_token_range(ast),
    }
}

fn anchor_column(subject: &str, ast: &AstFile) -> Option<TextRange> {
    let select_stmt = ast.select_stmt()?;
    let select_list = select_stmt.select_list()?;
    let mut fallback: Option<TextRange> = None;
    for item in select_list.items() {
        if item.alias().as_deref() == Some(subject) {
            return Some(item.alias_range().unwrap_or_else(|| item.range()));
        }
        if fallback.is_none() && item.expression_source_text().as_deref() == Some(subject) {
            fallback = Some(item.range());
        }
    }
    fallback
}

fn anchor_source(subject: &str, ast: &AstFile) -> Option<TextRange> {
    let select_stmt = ast.select_stmt()?;
    let from_clause = select_stmt.from_clause()?;
    // The subject is a source/model name — match on its last dotted
    // segment, since a `TableRef`'s own text carries the full ref call
    // (`smelt.ref('x')`) or a bare/qualified table name, not necessarily
    // the same spelling the diff used.
    let needle = subject.rsplit('.').next().unwrap_or(subject);
    for table_ref in from_clause.table_refs() {
        let text = table_ref.syntax().text().to_string();
        if text.contains(needle) {
            return Some(table_ref.syntax().text_range());
        }
    }
    for join in from_clause.joins() {
        let text = join.syntax().text().to_string();
        if text.contains(needle) {
            return Some(join.syntax().text_range());
        }
    }
    None
}

fn first_sql_token_range(ast: &AstFile) -> TextRange {
    let Some(select_stmt) = ast.select_stmt() else {
        return TextRange::empty(0.into());
    };
    select_stmt
        .syntax()
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| !t.kind().is_trivia())
        .map(|t| t.text_range())
        .unwrap_or_else(|| TextRange::empty(0.into()))
}

/// Build the `PropertyDowngrade` diagnostics for one model file
/// (`docs/specs/property_diff.md` §Diagnostics): one warning per story of
/// severity `risk` or `cost`, message = `"<lead>: <detail>"` — the same
/// line the text form prints — anchored per [`anchor_for`]. Returned as
/// `smelt_db::Diagnostic` values carrying raw `rowan::TextRange`s —
/// `Backend::to_lsp_diagnostic` converts them at the boundary, same as
/// every Salsa diagnostic.
pub fn diagnostics_for_model(model: &ModelDiff, ast: &AstFile) -> Vec<DbDiagnostic> {
    model
        .stories
        .iter()
        .filter(|s| matches!(s.severity, Severity::Risk | Severity::Cost))
        .map(|story| {
            let range = anchor_for(story.kind, &story.subject, ast);
            let message = format!("{}: {}", story.lead, story.detail);
            DbDiagnostic {
                severity: DiagnosticSeverity::Warning,
                message,
                range,
                code: Some(DiagnosticCode::PropertyDowngrade),
                data: None,
            }
        })
        .collect()
}

/// Build the code lens title for a shifted model, per §Surface "Editor":
/// the story-derived `diff_stories::lens_title` (`N risks, M costlier vs
/// <short ref>`, `changed vs <short ref>` with neither). `None` for an
/// unshifted model (`model` absent from `report.models`) — callers look
/// the model up themselves; this is the pure title primitive shared with
/// the parity gate and the CLI's `lens_title`.
pub fn lens_title_for(report: &DiffReport, model_name: &str) -> Option<String> {
    let model = report.models.iter().find(|m| m.model == model_name)?;
    Some(lens_title(model, &report.baseline))
}

/// Map every shifted model in `report` to its file path using `model_paths`
/// (model name -> file path, built by the caller from the loaded
/// workspace's `sql_files`), producing the lens-title and diagnostics maps
/// a [`ProjectDiffState`] refresh needs. Models not found in `model_paths`
/// (should not happen — every profiled model came from `sql_files`) are
/// silently skipped rather than panicking, per the fail-loud discipline's
/// "classify, don't crash" rule applied to an impossible-in-practice case.
pub fn derive_state_maps(
    report: &DiffReport,
    model_paths: &HashMap<String, PathBuf>,
    file_text: impl Fn(&Path) -> Option<String>,
) -> (
    HashMap<PathBuf, String>,
    HashMap<PathBuf, Vec<DbDiagnostic>>,
) {
    let mut lenses = HashMap::new();
    let mut diagnostics = HashMap::new();
    for model in &report.models {
        let Some(path) = model_paths.get(&model.model) else {
            continue;
        };
        let Some(text) = file_text(path) else {
            continue;
        };
        let parse = smelt_parser::parse(&text);
        let Some(ast) = AstFile::cast(parse.syntax()) else {
            continue;
        };
        lenses.insert(path.clone(), lens_title(model, &report.baseline));
        let diags = diagnostics_for_model(model, &ast);
        if !diags.is_empty() {
            diagnostics.insert(path.clone(), diags);
        }
    }
    (lenses, diagnostics)
}

/// The result of one [`refresh`] run.
pub enum RefreshOutcome {
    /// A diff was derived (git-resolvable workspace, working tree loaded).
    Report {
        commit: String,
        baseline: Arc<BaselineSide>,
        lenses: HashMap<PathBuf, String>,
        diagnostics: HashMap<PathBuf, Vec<DbDiagnostic>>,
    },
    /// The workspace is not a git work tree, or its baseline cannot be
    /// resolved (D8, non-git silence): no lens, no diagnostic, logged at
    /// `info` by the caller. Any previously cached state is cleared —
    /// unlike [`RefreshOutcome::Failed`], this is not a transient error.
    Silent(String),
    /// A transient derivation failure (e.g. the working tree's SQL fails
    /// to parse right now). The caller keeps whatever diff it last
    /// computed rather than showing nothing (§Surface "Editor", Δ3:
    /// "while a derivation is running, the editor shows the previously
    /// computed diff if one exists").
    Failed(String),
}

/// Run the property-diff pipeline for one project
/// (`docs/outcomes/20260905-property-diff/phases/07-plan.md` D1/D2/D7).
/// Pure over its arguments (no Salsa, no LSP client) so it can run inside
/// `spawn_blocking` — the caller is responsible for keeping this off the
/// request path (R6) and for snapshotting `overlays` from Salsa before
/// calling in.
///
/// `cached_baseline` is reused when the freshly re-resolved commit still
/// matches its own — re-resolution, not the `.git` watch, is what decides
/// cache validity (D2).
pub fn refresh(
    project_root: &Path,
    overlays: &BTreeMap<PathBuf, String>,
    cached_baseline: Option<Arc<BaselineSide>>,
) -> RefreshOutcome {
    let work = match work_side(project_root, overlays) {
        Ok(w) => w,
        Err(e) => return RefreshOutcome::Failed(e.to_string()),
    };

    // Always re-resolve (cheap: a few `git` subcommands) to decide whether
    // the cached baseline side is still valid — the correctness mechanism
    // (D2), not the `.git` watch that merely triggers this call promptly.
    let resolved = match smelt_core::baseline::resolve_baseline(project_root, None) {
        Ok(r) => r,
        Err(e) => return RefreshOutcome::Silent(e.to_string()),
    };

    let baseline: Arc<BaselineSide> = match &cached_baseline {
        Some(b) if b.resolved.commit == resolved.commit => Arc::clone(b),
        _ => match derive_baseline_side(project_root, None) {
            Ok(b) => Arc::new(b),
            Err(e) => return RefreshOutcome::Silent(e.to_string()),
        },
    };

    let report = derive_report(&work, &baseline);
    let model_paths: HashMap<String, PathBuf> = work
        .loaded
        .sql_files
        .iter()
        .map(|m| (m.canonical_path(), m.path.clone()))
        .collect();
    let (lenses, diagnostics) = derive_state_maps(&report, &model_paths, |path| {
        overlays
            .get(path)
            .cloned()
            .or_else(|| std::fs::read_to_string(path).ok())
    });

    RefreshOutcome::Report {
        commit: resolved.commit,
        baseline,
        lenses,
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ast(sql: &str) -> AstFile {
        let parse = smelt_parser::parse(sql);
        AstFile::cast(parse.syntax()).expect("valid SQL fixture must parse")
    }

    use smelt_logical::analysis::diff::{Cause, CauseKind};
    use smelt_logical::analysis::diff_stories::Story;

    /// D3: fails against a broken implementation that always falls back to
    /// the first SQL token — the asserted `(start, end)` offsets are the
    /// alias `renamed`'s, not the `SELECT` keyword's, so the two ranges
    /// disagree by the whole select list.
    #[test]
    fn anchor_column_subject_hits_the_select_item() {
        let sql = "SELECT a, b AS renamed FROM t";
        let ast = parse_ast(sql);
        let range = anchor_for(StoryKind::ColumnSemantics, "renamed", &ast);
        let expected_start = sql.find("renamed").unwrap() as u32;
        assert_eq!(u32::from(range.start()), expected_start);
        assert_eq!(
            u32::from(range.end()),
            expected_start + "renamed".len() as u32
        );
    }

    /// D3: fails against an implementation that substring-matches the cell
    /// subject anywhere in the SQL text (`amount` and `new_data` both
    /// appear nowhere in this fixture, so a substring-match implementation
    /// would either panic on `.unwrap()` or silently return an empty range
    /// at the wrong offset) — the correct behaviour is the model's first
    /// SQL token, the `SELECT` keyword at offset 0.
    #[test]
    fn anchor_cell_subject_falls_back_to_first_sql_token() {
        let sql = "SELECT a, b AS renamed FROM t";
        let ast = parse_ast(sql);
        let range = anchor_for(StoryKind::Technique, "amount@new_data", &ast);
        assert_eq!(u32::from(range.start()), 0);
        assert_eq!(u32::from(range.end()), "SELECT".len() as u32);
    }

    #[test]
    fn anchor_source_subject_hits_the_from_item() {
        let sql = "SELECT a FROM raw.users u JOIN raw.orders o ON u.id = o.user_id";
        let ast = parse_ast(sql);
        let range = anchor_for(StoryKind::Reads, "raw.orders", &ast);
        let expected_start = sql.find("JOIN raw.orders").unwrap() as u32;
        assert_eq!(u32::from(range.start()), expected_start);
    }

    /// `dependency` is the other source-subject kind (§Surface "Editor").
    #[test]
    fn dependency_story_anchors_at_the_from_item_too() {
        let sql = "SELECT a FROM raw.users u JOIN raw.orders o ON u.id = o.user_id";
        let ast = parse_ast(sql);
        let range = anchor_for(StoryKind::Dependency, "raw.orders", &ast);
        let expected_start = sql.find("JOIN raw.orders").unwrap() as u32;
        assert_eq!(u32::from(range.start()), expected_start);
    }

    #[test]
    fn anchor_falls_back_to_empty_range_when_there_is_no_sql() {
        let ast = parse_ast("");
        let range = anchor_for(StoryKind::MaintenanceLost, "", &ast);
        assert_eq!(range, TextRange::empty(0.into()));
    }

    fn story(
        kind: StoryKind,
        severity: Severity,
        subject: &str,
        lead: &str,
        detail: &str,
    ) -> Story {
        Story {
            kind,
            severity,
            subject: subject.to_string(),
            lead: lead.to_string(),
            detail: detail.to_string(),
            changes: Vec::new(),
        }
    }

    fn model_diff_with_stories(stories: Vec<Story>) -> ModelDiff {
        ModelDiff {
            model: "m".to_string(),
            cause: Cause {
                kind: CauseKind::Edited,
                of: vec!["m".to_string()],
                reason: None,
            },
            changes: Vec::new(),
            stories,
        }
    }

    /// A `risk` story folding several downgrades still yields exactly one
    /// `PropertyDowngrade` diagnostic, whose message is `"<lead>: <detail>"`
    /// (§Surface "Diagnostics") — not one diagnostic per folded change.
    #[test]
    fn one_diagnostic_per_risk_or_cost_story() {
        let ast = parse_ast("SELECT a FROM t");
        let model = model_diff_with_stories(vec![
            story(
                StoryKind::RowsMayDuplicate,
                Severity::Risk,
                "",
                "Rows may be duplicated",
                "A join can now match more than one row per (id).",
            ),
            story(
                StoryKind::Reads,
                Severity::Cost,
                "",
                "Reads more per run",
                "Each run now reads all history of t.",
            ),
            story(StoryKind::Schema, Severity::Info, "", "Schema", "Adds b."),
        ]);
        let diags = diagnostics_for_model(&model, &ast);
        assert_eq!(
            diags.len(),
            2,
            "one diagnostic per risk/cost story: {diags:?}"
        );
        assert!(diags.iter().any(|d| d.message
            == "Rows may be duplicated: A join can now match more than one row per (id)."));
        assert!(diags
            .iter()
            .any(|d| d.message == "Reads more per run: Each run now reads all history of t."));
    }

    /// §Surface "Editor": a `schema` story is `info` severity and never a
    /// diagnostic; `column_semantics` anchors at the SELECT item;
    /// `reads`/`dependency` anchor at the FROM/JOIN item; a story with an
    /// empty subject (`rows_may_duplicate`) anchors at the first SQL token.
    #[test]
    fn story_subject_anchors_column_source_or_first_token() {
        let sql =
            "SELECT id, amount AS total FROM raw.orders o JOIN raw.users u ON o.user_id = u.id";
        let ast = parse_ast(sql);
        let model = model_diff_with_stories(vec![
            story(
                StoryKind::Schema,
                Severity::Info,
                "",
                "Schema",
                "Adds total.",
            ),
            story(
                StoryKind::ColumnSemantics,
                Severity::Risk,
                "total",
                "Column now nondeterministic",
                "total is now Run-nondeterministic (was Clean).",
            ),
            story(
                StoryKind::Reads,
                Severity::Cost,
                "raw.orders",
                "Reads more per run",
                "Each run now reads all history of raw.orders.",
            ),
            story(
                StoryKind::RowsMayDuplicate,
                Severity::Risk,
                "",
                "Rows may be duplicated",
                "A join can now match more than one row per (id).",
            ),
        ]);
        let diags = diagnostics_for_model(&model, &ast);
        // The `schema` story is info-severity: no diagnostic for it.
        assert!(!diags.iter().any(|d| d.message.starts_with("Schema:")));
        assert_eq!(diags.len(), 3);

        let column_diag = diags
            .iter()
            .find(|d| d.message.starts_with("Column now nondeterministic:"))
            .expect("column_semantics story must yield a diagnostic");
        let expected_start = sql.find("total").unwrap() as u32;
        assert_eq!(u32::from(column_diag.range.start()), expected_start);

        let source_diag = diags
            .iter()
            .find(|d| d.message.starts_with("Reads more per run:"))
            .expect("reads story must yield a diagnostic");
        let source_text =
            &sql[usize::from(source_diag.range.start())..usize::from(source_diag.range.end())];
        assert!(
            source_text.contains("raw.orders"),
            "reads story must anchor at the raw.orders FROM item, got {source_text:?}"
        );

        let first_token_diag = diags
            .iter()
            .find(|d| d.message.starts_with("Rows may be duplicated:"))
            .expect("rows_may_duplicate story must yield a diagnostic");
        assert_eq!(u32::from(first_token_diag.range.start()), 0);
    }
}
