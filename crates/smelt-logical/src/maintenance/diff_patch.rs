//! `diff_patch` admission — the reconciliation/idempotent-rerun write pattern
//! (`docs/specs/incremental_models.md` §"The write-pattern set is open" →
//! "`diff_patch` — compute, diff, write only the difference"): the candidate
//! rows for a slice are computed, diffed against the slice's stored state,
//! and only the difference is written — inserting rows absent from storage,
//! updating stored rows whose compared columns differ, and deleting stored
//! rows absent from a *complete* candidate set.
//!
//! Three facts gate it, each fail-closed:
//! - **row identity** (`RowIdentity::Key`) for the insert/update legs' diff
//!   join — a [`RowIdentity::WholeRow`] cell refuses outright
//!   ([`DiffPatchRefusal::IdentityRequired`]); there is no keyless shape for
//!   this pattern (unlike `staged_candidate`'s own keyless residue, see that
//!   emitter's doc comment).
//! - **change comparability** (P3, `model_properties.md` §"Change
//!   comparability") over the written columns, for the update leg — reused
//!   verbatim from [`super::choice::resolve_write_suppression`] rather than
//!   re-derived here (`CLAUDE.md` §"Maintenance-plan purity"). An
//!   incomparable (or unproven) compared column refuses the whole pattern
//!   ([`DiffPatchRefusal::ComparabilityRequired`]) rather than falling back
//!   to an unconditional update leg: an unconditional overwrite of every
//!   matched row is just delete+insert with extra steps, which defeats
//!   `diff_patch`'s entire reason for existing (write only the difference).
//!   A caller whose column is incomparable must fall back to a *different*
//!   write pattern, not get a degraded one out of this function.
//! - **slice completeness**, for the delete leg only (§"The repair family"'s
//!   own premise, reused rather than re-proven here). Determining whether a
//!   given `candidate_select` is complete over its slice is context-specific
//!   — a repair family's per-group bounded read has one completeness
//!   argument (key temporal locality, §"Key temporal locality"), a region
//!   recompute's windowed read has a different one (the write-window clamp
//!   itself) — so this function takes the verdict as a caller-supplied
//!   `Result<(), String>` rather than re-deriving a proof that differs by
//!   caller. Missing completeness degrades the admitted verdict's delete leg
//!   to [`DeleteLeg::Omitted`] — a stated reduced-capability admission, never
//!   a silently dropped delete.

use crate::analysis::walk::ColumnComparability;

use super::choice::resolve_write_suppression;
use super::{RowIdentity, RowIdentityVerdict};

/// Whether the admitted `diff_patch` write includes the delete leg (removing
/// stored rows absent from the candidate set).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteLeg {
    /// The candidate set is proven complete over the slice — the delete leg
    /// is sound and included.
    Complete,
    /// Slice completeness could not be proven — the delete leg is omitted;
    /// `why` carries the caller's own reason, so a consumer (`smelt explain`)
    /// can show why the pattern degraded rather than only ever seeing the
    /// reduced shape.
    Omitted { why: String },
}

/// Why `diff_patch` was refused outright — no partial admission, never a
/// silent downgrade to a different pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffPatchRefusal {
    /// No proven row identity (`RowIdentity` is `WholeRow`) — the diff join
    /// has nothing to address rows by.
    IdentityRequired { why: String },
    /// At least one compared column is not proven comparable across runs (or
    /// no row identity was proven at all) —
    /// [`super::choice::resolve_write_suppression`]'s own refusal reason,
    /// verbatim.
    ComparabilityRequired { why: String },
}

/// The admitted `diff_patch` verdict: the diff-join key, the columns the
/// update leg compares, and whether the delete leg is sound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedDiffPatch {
    pub key: Vec<String>,
    pub compared_columns: Vec<String>,
    pub delete_leg: DeleteLeg,
}

/// Admit (or fail-closed refuse) `diff_patch` for a cell whose column group
/// is `group_columns`, given the walk's comparability verdict, the cell's
/// proven row identity, and a caller-supplied slice-completeness proof for
/// the delete leg (`Ok(())` when complete, `Err(why)` otherwise).
pub fn admit_diff_patch(
    group_columns: &[String],
    comparability: &[ColumnComparability],
    row_identity: &RowIdentityVerdict,
    slice_complete: Result<(), String>,
) -> Result<AdmittedDiffPatch, DiffPatchRefusal> {
    let key = match &row_identity.identity {
        RowIdentity::Key(key) => key.clone(),
        RowIdentity::WholeRow => {
            return Err(DiffPatchRefusal::IdentityRequired {
                why: "no proven row identity (P2 verdict is WholeRow) — diff_patch's diff join \
                      cannot address individual rows without a proven key"
                    .to_string(),
            });
        }
    };

    let compared_columns =
        match resolve_write_suppression(group_columns, comparability, row_identity) {
            super::choice::WriteSuppression::Suppressed { compared_columns } => compared_columns,
            super::choice::WriteSuppression::Unconditional { why } => {
                return Err(DiffPatchRefusal::ComparabilityRequired { why });
            }
        };

    let delete_leg = match slice_complete {
        Ok(()) => DeleteLeg::Complete,
        Err(why) => DeleteLeg::Omitted { why },
    };

    Ok(AdmittedDiffPatch {
        key,
        compared_columns,
        delete_leg,
    })
}
