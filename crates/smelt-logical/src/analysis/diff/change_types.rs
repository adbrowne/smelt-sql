use super::*;

pub(super) fn to_json<T: Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// The items of `a` that occur more times in `a` than in `b`, one entry per
/// excess occurrence — a multiset difference. `DerivedFd` has no `Ord`/
/// `Hash`, so this is a small O(n²) linear scan rather than a `BTreeMap`
/// one; functional-dependency lists are short (G7,
/// `docs/outcomes/20260905-property-diff` fix round 1 — a plain
/// `Vec::contains` membership check silently drops a duplicate's removal).
pub(super) fn multiset_excess<T: Clone + PartialEq>(a: &[T], b: &[T]) -> Vec<T> {
    let mut b_remaining: Vec<bool> = vec![true; b.len()];
    let mut excess = Vec::new();
    for item in a {
        if let Some(slot) = b_remaining
            .iter_mut()
            .zip(b.iter())
            .find(|(available, candidate)| **available && *candidate == item)
        {
            *slot.0 = false;
        } else {
            excess.push(item.clone());
        }
    }
    excess
}

/// One reported difference (`docs/specs/property_diff.md` §Surface "JSON").
#[derive(Debug, Clone, Serialize)]
pub struct Change {
    pub dimension: Dimension,
    pub subject: String,
    pub direction: Direction,
    pub old: Option<Value>,
    pub new: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip)]
    pub kind: ChangeKind,
}

impl Change {
    /// Build a [`Change`] from a typed [`ChangeKind`], deriving `dimension`,
    /// `subject`, `direction`, `old`/`new`, and `reason` from it — the one
    /// constructor every [`Change`] goes through, so a caller (including
    /// `story_coverage`'s generative gate,
    /// `docs/specs/property_diff.md` §Constraints item 11) can build a
    /// [`Change`] from a [`ChangeKind`] and get the *real* derived fields
    /// rather than hand-rolling them.
    pub fn from_kind(kind: ChangeKind) -> Self {
        Change {
            dimension: kind.dimension(),
            subject: kind.subject(),
            direction: kind.direction(),
            old: kind.old_json(),
            new: kind.new_json(),
            reason: kind.reason(),
            kind,
        }
    }
}

/// Which kind of cause a shifted model's entry carries
/// (`docs/specs/property_diff.md` §"Attribution").
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CauseKind {
    Edited,
    Added,
    Removed,
    Downstream,
}

/// A shifted model's attribution (`docs/specs/property_diff.md`
/// §"Attribution").
#[derive(Debug, Clone, Serialize)]
pub struct Cause {
    pub kind: CauseKind,
    pub of: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// One shifted model's report (`docs/specs/property_diff.md` §Surface
/// "JSON").
#[derive(Debug, Clone, Serialize)]
pub struct ModelDiff {
    pub model: String,
    pub cause: Cause,
    pub changes: Vec<Change>,
    /// The severity-ranked narration of `changes`
    /// (`docs/specs/property_diff.md` §"Stories"), produced once by
    /// [`crate::analysis::diff_stories::narrate`] and carried here so every
    /// renderer reads the same value rather than re-folding the changes
    /// itself (§Constraints item 10, "Narration single ownership").
    pub stories: Vec<crate::analysis::diff_stories::Story>,
}

/// The diff's summary counts (`docs/specs/property_diff.md` §Surface
/// "JSON").
#[derive(Debug, Clone, Default, Serialize)]
pub struct DiffSummary {
    pub downgrades: usize,
    pub upgrades: usize,
    pub neutral: usize,
    pub shifted_models: usize,
}

/// The whole property diff (`docs/specs/property_diff.md` §Surface "JSON").
#[derive(Debug, Clone, Serialize)]
pub struct PropertyDiff {
    pub models: Vec<ModelDiff>,
    pub summary: DiffSummary,
}

/// The `baseline` object of the JSON schema
/// (`docs/specs/property_diff.md` §Surface "JSON").
#[derive(Debug, Clone, Serialize)]
pub struct BaselineInfo {
    #[serde(rename = "ref")]
    pub r#ref: String,
    pub commit: String,
    pub resolved_as: String,
}

impl From<&smelt_core::baseline::ResolvedBaseline> for BaselineInfo {
    fn from(resolved: &smelt_core::baseline::ResolvedBaseline) -> Self {
        BaselineInfo {
            r#ref: resolved.requested.clone(),
            commit: resolved.commit.clone(),
            resolved_as: match resolved.resolved_as {
                smelt_core::baseline::ResolvedAs::Explicit => "explicit".to_string(),
                smelt_core::baseline::ResolvedAs::MergeBase => "merge_base".to_string(),
            },
        }
    }
}

/// The full `smelt explain --diff` report — top-level key order here IS the
/// §Surface "JSON" schema's top-level key order
/// (`docs/specs/property_diff.md` §Surface "JSON"). Every renderer
/// (`analysis::diff_render`, and later the Markdown/LSP consumers) reads
/// this value; none re-derives or re-sorts `models`
/// (`docs/outcomes/20260905-property-diff/phases/05-plan.md` D5).
#[derive(Debug, Clone, Serialize)]
pub struct DiffReport {
    pub baseline: BaselineInfo,
    pub edited_files: Vec<String>,
    pub summary: DiffSummary,
    /// The report's one-line summary (`docs/specs/property_diff.md`
    /// §"Stories" "Headline"), derived from `models`' own stories by
    /// [`crate::analysis::diff_stories::headline`] — recomputed by
    /// [`DiffReport::narrow_to`] whenever `models` is narrowed, so it always
    /// counts the reported set.
    pub headline: String,
    pub models: Vec<ModelDiff>,
}

impl DiffReport {
    /// Assemble a [`DiffReport`] from a computed [`PropertyDiff`] plus the
    /// baseline/edited-file facts the caller (a later phase, git-aware)
    /// already resolved. `diff_profiles`'s pure `models`/`summary` are
    /// carried unchanged; this is presentation-envelope assembly, not a
    /// second diff.
    pub fn new(baseline: BaselineInfo, edited_files: Vec<String>, diff: PropertyDiff) -> Self {
        let mut report = DiffReport {
            baseline,
            edited_files,
            summary: diff.summary,
            headline: String::new(),
            models: diff.models,
        };
        report.headline = crate::analysis::diff_stories::headline(&report);
        report
    }

    /// Narrow the REPORTED set to `selected` model names
    /// (`docs/specs/property_diff.md` §Surface "`--select`", Δ2): `--diff`'s
    /// `--select` restricts what is printed, not what was compared —
    /// `diff_profiles` (and therefore attribution) already ran over every
    /// model before this is called, so a retained entry's `cause` is
    /// unaffected. The summary counts are recomputed from the retained
    /// `models` so they always match what is actually printed
    /// (`docs/outcomes/20260905-property-diff/phases/05-plan.md` D6).
    /// Single-owned here (not in the CLI) so Phase 6's Markdown renderer
    /// and Phase 7's LSP reuse the same narrowing rather than each
    /// reimplementing retain-and-recount.
    pub fn narrow_to(&mut self, selected: &BTreeSet<String>) {
        self.models.retain(|m| selected.contains(&m.model));
        let mut summary = DiffSummary {
            shifted_models: self.models.len(),
            ..Default::default()
        };
        for m in &self.models {
            for c in &m.changes {
                match c.direction {
                    Direction::Downgrade => summary.downgrades += 1,
                    Direction::Upgrade => summary.upgrades += 1,
                    Direction::Neutral => summary.neutral += 1,
                }
            }
        }
        self.summary = summary;
        self.headline = crate::analysis::diff_stories::headline(self);
    }
}
