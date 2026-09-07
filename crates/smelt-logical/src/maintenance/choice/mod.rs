//! Technique choice among admissible alternatives — the override ladder
//! (`defaults.prefer` → `cells[].prefer` → `cells[].technique`, narrower
//! scope winning) plus the cost-model hook `smelt bakeoff` measures into
//! (`incremental_models.md` §Surface "Frontmatter", §Semantics
//! "Interchangeability and choice", §Design "Offline cost measurement is
//! first-class").
//!
//! `derive_maintenance_plan` (`derive.rs`) admits exactly one [`Technique`]
//! per cell today — there is no multi-technique admission set inside the
//! pure plan yet (see that module's doc comment). The second live
//! alternative that exists for every cell whose admitted technique realizes
//! the top-right/bottom-left corners (fold-a-delta, column-scoped
//! re-derivation) is the always-admissible whole-region recompute
//! (`Technique::DeleteInsert`): a recompute is contract-agnostic and
//! unconditionally valid over replayable input
//! (`incremental_models.md` §Semantics "The plan matrix"). This module treats
//! `{the cell's own admitted technique, RegionRecompute}` as the resolvable
//! set and applies the override ladder over it — pure data in, pure data
//! out, per the "Maintenance-plan purity" invariant (root `CLAUDE.md`).
//!
//! A `technique:` pin naming a technique outside that resolvable set is an
//! admission failure ([`ChoiceRefusal`]), never a silent override — the
//! spec's "a pin bypasses the cost model, never admission."

use smelt_core::config::{
    CellTechnique, MaintenanceCellConfig, MaintenanceDefaults, TechniquePreference,
};

use crate::analysis::walk::{ColumnComparability, Comparability};

use super::diff_patch;
use super::{PlanCell, RowIdentity, RowIdentityVerdict, Technique, Trigger};

/// The technique the ladder resolves to for one cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChosenTechnique {
    /// The cell's own admitted technique (fold family / column-scoped merge
    /// / in-place update — whichever `derive_maintenance_plan` picked).
    Admitted(Technique),
    /// The always-available whole-region recompute (`DELETE`+`INSERT`),
    /// chosen either because it is the only resolvable member or because
    /// the ladder/cost-model preferred it.
    RegionRecompute,
    /// The `diff_patch` write pattern (`incremental_models.md` §"The
    /// write-pattern set is open"): `recompute` is the underlying
    /// recompute-family technique the candidate is drawn from
    /// (`Technique::DeleteInsert` or `Technique::PerGroupRecompute`, or
    /// `DeleteInsert` as the region-recompute default when the trigger has
    /// no admitted cell), and `delete_leg` is the delete-leg admission
    /// verdict ([`diff_patch::DeleteLeg`]) — carried here rather than
    /// re-resolved by every consumer, per `CLAUDE.md` §"Maintenance-plan
    /// purity".
    ///
    /// **Known simplification (this phase's own scope boundary):**
    /// `resolve_cell_choice` threads exactly one slice-completeness premise
    /// through today — `Technique::PerGroupRecompute`'s own bounded-slice
    /// admission (the repair family's key-temporal-locality proof) — so this
    /// variant carries `delete_leg: DeleteLeg::Complete` only over that
    /// recompute technique; every other recompute (region `DeleteInsert`)
    /// still carries `DeleteLeg::Omitted { .. }` until that technique's own
    /// completeness proof is threaded through here too.
    DiffPatch {
        recompute: Technique,
        delete_leg: diff_patch::DeleteLeg,
    },
}

/// Which kind of hard pin a [`ChoiceRefusal`] names: the `cells[].technique`
/// fold/recompute/rederive-columns pin, or a `cells[].write` open-registry
/// pin (already validated against the registry itself by
/// [`super::resolve_write_pin`] — this refusal fires one level deeper, when
/// the *validated* pattern's [`super::WriteSelection`] still isn't what this
/// cell's derived plan actually admits, e.g. `write: keyed` resolves fine
/// against the registry for an identity-bearing output but this particular
/// trigger's cell admitted `Technique::ColumnScopedMerge`, not
/// `KeyedFold`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinnedRequest {
    Technique(CellTechnique),
    Write(String),
}

/// Why a requested technique choice could not be honoured: `cells[].technique`
/// (or a soft `prefer`, when it disagrees with every resolvable member) names
/// a technique outside `{the cell's own admitted technique, RegionRecompute}`
/// — a pin bypasses the cost model, never admission
/// (`incremental_models.md` §Surface "Frontmatter"). A `cells[].write` pin
/// that resolves in the open registry but whose selected [`super::
/// WriteSelection`] this cell's derived plan does not admit refuses the
/// same way — never a silent downgrade to a different technique than the
/// pin named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceRefusal {
    pub trigger: String,
    pub pinned: PinnedRequest,
    pub why: String,
}

impl std::fmt::Display for ChoiceRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "MaintenanceUnboundedFootprint: pinned technique '{:?}' for {} is not in the \
             admissible set — {}",
            self.pinned, self.trigger, self.why
        )
    }
}

/// The effective per-cell override once the ladder narrows: `cells[].technique`
/// (a hard pin) if present, else `cells[].prefer` if present, else
/// `defaults.prefer` — narrower scope always wins over broader
/// (`incremental_models.md` §Surface "Frontmatter": "The override ladder is
/// `defaults.prefer` → `cells[].prefer` → `cells[].technique`, narrower
/// scope winning").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EffectiveOverride {
    /// Soft bias — the cost model may still choose a different resolvable
    /// technique. `None`/`Auto` both mean "no soft bias": the resolver falls
    /// through to its own deterministic default.
    pub prefer: Option<TechniquePreference>,
    /// Hard pin — bypasses the cost model, never bypasses admission.
    pub technique: Option<CellTechnique>,
}

/// Match one `maintenance.cells[]` entry against `trigger_address` (the
/// cell's `on:` value — a source address or the literal `backfill`) and
/// `group_columns` (any member of the cell's derived column group — the
/// `cells[].columns` match is "names any member", not "equals exactly",
/// per §Surface "Frontmatter").
fn matching_cell<'a>(
    cells: &'a [MaintenanceCellConfig],
    trigger_address: &str,
    group_columns: &[String],
) -> Option<&'a MaintenanceCellConfig> {
    cells.iter().find(|c| {
        c.on == trigger_address
            && c.columns
                .iter()
                .any(|col| group_columns.iter().any(|g| g == col))
    })
}

/// A HARD `cells[].technique` pin naming `on: trigger_address` whose
/// `columns` intersect NONE of `sibling_group_columns` — every derived
/// column group the trigger's own admitted cells actually carry
/// (`MaintenancePlan::cells_for`, `docs/plans/20260808-membership-
/// sensitivity.md` Phase 3). A trigger commonly derives MULTIPLE sibling
/// cells, one per membership-sensitive column group a shared join admits;
/// a pin whose `columns` name none of them at all is not "no override for
/// this cell" (silent, fine) — it is a dangling/misconfigured pin that
/// will never be consulted by anything, which fail-loud discipline
/// (root `CLAUDE.md` §"Fail-loud discipline") says must surface, not
/// vanish.
///
/// **Only a hard `technique:` pin is checked here.** A soft `cells[].prefer`
/// naming the same unaddressed columns is not an error — `prefer` is
/// documented to never refuse even when it names a technique outside the
/// resolvable set (`resolve_cell_choice`'s own doc comment: "falling back
/// silently to the deterministic default when the preferred family isn't
/// resolvable"); the SAME silent-fallback contract extends naturally to a
/// `prefer` that fails to address any sibling group at all — there is
/// nothing new to refuse about that case that plain "prefer had no
/// admissible target" didn't already cover. A `cells[].write` pin has its
/// own, separately-checked matching rule
/// (`smelt-db`'s `matching_write_pin`/`write_pin_diagnostics`, unaffected by
/// this function) — not this function's concern.
///
/// A `columns: []` entry (the whole-row `{*}` trigger shape,
/// `NewData`/`Backfill`) is never flagged — [`matching_cell`]'s own
/// "any column member" rule never matches an empty `columns` list either,
/// so an empty-`columns` pin addresses its trigger by `on:` alone and is
/// out of this per-column-group check's scope.
pub fn unaddressed_technique_pin<'a>(
    cells: &'a [MaintenanceCellConfig],
    trigger_address: &str,
    sibling_group_columns: &[Vec<String>],
) -> Option<&'a MaintenanceCellConfig> {
    cells.iter().find(|c| {
        c.on == trigger_address
            && c.technique.is_some()
            && !c.columns.is_empty()
            && !sibling_group_columns
                .iter()
                .any(|group| c.columns.iter().any(|col| group.contains(col)))
    })
}

/// Resolve the effective override for one cell, applying the narrower-wins
/// ladder. `cells` is the model's `maintenance.cells[]` frontmatter (already
/// scoped to this model — there is no project-level default for the
/// technique ladder, unlike `scan_bounds`).
pub fn effective_override(
    defaults: Option<&MaintenanceDefaults>,
    cells: &[MaintenanceCellConfig],
    trigger_address: &str,
    group_columns: &[String],
) -> EffectiveOverride {
    let broad_prefer = defaults.and_then(|d| d.prefer);
    let narrow = matching_cell(cells, trigger_address, group_columns);
    EffectiveOverride {
        prefer: narrow.and_then(|c| c.prefer).or(broad_prefer),
        technique: narrow.and_then(|c| c.technique),
    }
}

/// Human-readable trigger label for diagnostics — mirrors the `{trigger:?}`
/// convention `derive.rs`'s own refusals use.
fn trigger_label(trigger: &Trigger) -> String {
    format!("{trigger:?}")
}

/// Whether `technique` is a member of the cell's resolvable set: the cell's
/// own admitted technique (only when the backend can actually run it) or the
/// always-available region recompute.
fn admits(
    pin: CellTechnique,
    admitted: Option<&Technique>,
    backend_supports_column_scoped_merge: bool,
) -> bool {
    match pin {
        CellTechnique::Recompute => true,
        CellTechnique::Fold => matches!(
            admitted,
            Some(Technique::KeyedFold) | Some(Technique::InPlaceUpdate)
        ),
        CellTechnique::RederiveColumns => {
            admitted == Some(&Technique::ColumnScopedMerge) && backend_supports_column_scoped_merge
        }
        // Never reached: the caller only calls `admits` after narrowing
        // `overrides.technique` to a family variant — `Suppress`/
        // `Unconditional` are the orthogonal write-suppression pin,
        // resolved by `resolve_write_variant`, not a family pin this
        // function's resolvable set contains.
        CellTechnique::Suppress | CellTechnique::Unconditional => false,
    }
}

/// Whether a validated [`super::WriteSelection`] (the resolved shape of a
/// `cells[].write` pin — already checked against the open registry and the
/// backend's write capabilities by [`super::resolve_write_pin`]) is a member
/// of this cell's resolvable set: the always-available region recompute, or
/// the cell's own admitted (and, for `ColumnScopedMerge`, backend-live)
/// technique when it matches the selection's `Technique` exactly.
fn admits_write_selection(
    selection: super::WriteSelection,
    admitted: Option<&Technique>,
    backend_supports_column_scoped_merge: bool,
) -> bool {
    match selection {
        super::WriteSelection::RegionRecompute => true,
        super::WriteSelection::Technique(Technique::ColumnScopedMerge) => {
            admitted == Some(&Technique::ColumnScopedMerge) && backend_supports_column_scoped_merge
        }
        super::WriteSelection::Technique(t) => admitted == Some(&t),
        super::WriteSelection::DiffPatch => matches!(
            admitted,
            None | Some(Technique::DeleteInsert) | Some(Technique::PerGroupRecompute)
        ),
    }
}

/// Resolve which technique executes for `trigger`, given the plan, the
/// effective override (already narrowed by [`effective_override`]), an
/// optional already-validated `cells[].write` pin
/// ([`super::resolve_write_pin`]'s `Ok` result — this function never
/// re-validates the pin against the registry or backend capabilities, only
/// against what THIS trigger's cell actually admits), and whether the target
/// backend can run a column-scoped `MERGE` at all.
///
/// Mirrors `incremental_models.md` §"Per-cell admission": a `technique:` pin
/// bypasses the cost model, **never** admission — pinning a technique the
/// resolvable set does not contain is a hard, fail-loud [`ChoiceRefusal`],
/// not a silent fallback to `RegionRecompute`. A soft `prefer` never
/// refuses: it only nudges the choice among what IS resolvable, falling back
/// silently to the deterministic default when the preferred family isn't
/// resolvable (that is what "soft" means — `cells[].prefer`'s doc comment:
/// "the cost model may still choose a different admissible technique").
/// Absent any override, the cell's own admitted+live technique is preferred
/// over region recompute (the point of admitting it at all); otherwise
/// region recompute is the safe default.
///
/// A `write_pin` is consulted **before** the `cells[].technique`/`prefer`
/// ladder and, when present, decides the cell alone: `incremental_models.md`
/// §"Per-cell write addressing" names `cells[].write` the addressing-level
/// pin, one layer more specific than the fold-vs-recompute `technique`
/// ladder, so a cell carrying both pins is resolved by its `write` pin — the
/// `technique`/`prefer` ladder is not consulted for that cell in that case.
/// `resolve_write_pin`'s own registry/capability/equivalence checks already
/// ran by the time a `Some(pattern)` reaches here; this only asks whether
/// the validated pattern's [`super::WriteSelection`] is realizable by THIS
/// trigger's own derived plan cell — a validated-but-structurally-mismatched
/// pin (e.g. `write: keyed` validated against the output's declared facts,
/// but this particular trigger's cell happens to have admitted
/// `ColumnScopedMerge`, not `KeyedFold`) still refuses, never silently
/// substitutes a different technique than the one named.
///
/// **`cell` is the caller's responsibility to pick.** This function no
/// longer looks a cell up from a whole [`MaintenancePlan`] by `trigger`
/// alone — a trigger commonly derives MULTIPLE sibling cells, one per
/// membership-sensitive column group a shared join admits
/// (`docs/plans/20260808-membership-sensitivity.md` Phase 1), and picking
/// "the" cell by trigger alone (`MaintenancePlan::cell_for`'s own
/// first-match semantics) silently evaluated every override against only
/// the FIRST sibling, regardless of which sibling's own `columns` an
/// override actually named (Phase 3's fix). The caller must resolve the
/// correct sibling itself — typically by iterating
/// [`MaintenancePlan::cells_for`] and matching each candidate's own derived
/// column group against the override, mirroring [`effective_override`]'s
/// own per-group `matching_cell` logic — and pass that specific cell (or
/// `None`, when the trigger has no admitted cell at all: an override still
/// resolves against `{recompute}` alone, per the `None` arms below).
pub fn resolve_cell_choice(
    cell: Option<&PlanCell>,
    trigger: &Trigger,
    overrides: &EffectiveOverride,
    write_pin: Option<&'static super::WritePattern>,
    backend_supports_column_scoped_merge: bool,
) -> Result<ChosenTechnique, ChoiceRefusal> {
    let admitted_technique = cell.map(|c| &c.technique);
    let live_technique = admitted_technique.filter(|t| match t {
        Technique::ColumnScopedMerge => backend_supports_column_scoped_merge,
        _ => true,
    });

    if let Some(pattern) = write_pin {
        let selection = pattern.selects();
        return if admits_write_selection(
            selection.clone(),
            admitted_technique,
            backend_supports_column_scoped_merge,
        ) {
            match selection {
                super::WriteSelection::RegionRecompute => Ok(ChosenTechnique::RegionRecompute),
                super::WriteSelection::Technique(_) => {
                    Ok(ChosenTechnique::Admitted(*admitted_technique.expect(
                        "admits_write_selection already proved `admitted_technique` is \
                         Some for this pin",
                    )))
                }
                // `diff_patch` selects the diff-then-patch pattern over
                // whichever recompute-family technique the cell admitted
                // (or the region-recompute default when the trigger has no
                // admitted cell at all — `admits_write_selection`'s own
                // `None` arm). See `ChosenTechnique::DiffPatch`'s doc
                // comment for why `delete_leg` is always `Omitted` here:
                // threading the real slice-completeness proof through this
                // admission layer is routing/lowering's job (a later
                // phase), not this one's.
                super::WriteSelection::DiffPatch => {
                    let recompute = admitted_technique
                        .copied()
                        .unwrap_or(Technique::DeleteInsert);
                    // Slice-completeness premise (`incremental_models.md`
                    // §"`diff_patch` — compute, diff, write only the
                    // difference"): a `PerGroupRecompute` recompute already
                    // discharged the repair family's own key-temporal-
                    // locality premise at admission
                    // (`repair::admit_per_group_recompute`'s bounded
                    // per-group slice), which is exactly diff_patch's own
                    // completeness argument for that slice — so the delete
                    // leg is sound. The region `DeleteInsert` default's own
                    // completeness argument is its write-window clamp
                    // itself: the candidate a region recompute writes IS
                    // the model's entire admitted state over the clamped
                    // write range, so a stored row inside that range absent
                    // from the candidate really has departed, never merely
                    // unscanned — the same premise `resolve_repair_write`'s
                    // caller threads for the membership-recompute lowering
                    // (`docs/outcomes/20260815-definition-delta-migrate/
                    // phases/12-plan.md`). Every OTHER recompute has no such
                    // premise threaded through this admission layer yet, so
                    // its delete leg stays omitted with a stated reason
                    // rather than silently assumed complete.
                    let delete_leg = if matches!(
                        recompute,
                        Technique::PerGroupRecompute | Technique::DeleteInsert
                    ) {
                        diff_patch::DeleteLeg::Complete
                    } else {
                        diff_patch::DeleteLeg::Omitted {
                            why: "slice-completeness proof is not yet threaded through \
                                  resolve_cell_choice for this recompute technique — only \
                                  PerGroupRecompute's key-temporal-locality premise and the \
                                  region DeleteInsert default's write-window clamp (both \
                                  already proven at admission) discharge it here"
                                .to_string(),
                        }
                    };
                    Ok(ChosenTechnique::DiffPatch {
                        recompute,
                        delete_leg,
                    })
                }
            }
        } else {
            Err(ChoiceRefusal {
                trigger: trigger_label(trigger),
                pinned: PinnedRequest::Write(pattern.name.to_string()),
                why: format!(
                    "the derived plan's resolvable set for this cell is {{{}}} — a write pin \
                     bypasses the cost model, never admission",
                    resolvable_set_label(admitted_technique, backend_supports_column_scoped_merge)
                ),
            })
        };
    }

    // `Suppress`/`Unconditional` are the orthogonal write-suppression pin —
    // never a family pin — so they fall through to the soft `prefer`/
    // structural default below for FAMILY choice; [`resolve_write_variant`]
    // is where they actually take effect.
    if let Some(
        pin @ (CellTechnique::Fold | CellTechnique::Recompute | CellTechnique::RederiveColumns),
    ) = overrides.technique
    {
        return if admits(
            pin,
            admitted_technique,
            backend_supports_column_scoped_merge,
        ) {
            match pin {
                CellTechnique::Recompute => Ok(ChosenTechnique::RegionRecompute),
                CellTechnique::Fold | CellTechnique::RederiveColumns => {
                    Ok(ChosenTechnique::Admitted(*admitted_technique.expect(
                        "admits() already proved `admitted_technique` is Some for this pin",
                    )))
                }
                // The outer `if let` guard already narrowed `pin` to one of
                // the three arms above — `Suppress`/`Unconditional` never
                // reach here.
                CellTechnique::Suppress | CellTechnique::Unconditional => {
                    unreachable!("narrowed to a family variant by the outer `if let` guard")
                }
            }
        } else {
            Err(ChoiceRefusal {
                trigger: trigger_label(trigger),
                pinned: PinnedRequest::Technique(pin),
                why: format!(
                    "the derived plan's resolvable set for this cell is {{{}}} — a pin \
                     bypasses the cost model, never admission",
                    resolvable_set_label(admitted_technique, backend_supports_column_scoped_merge)
                ),
            })
        };
    }

    // No hard pin: a soft `prefer` nudges among what IS resolvable, but
    // never refuses.
    match overrides.prefer {
        Some(TechniquePreference::Recompute) => Ok(ChosenTechnique::RegionRecompute),
        _ => match live_technique {
            Some(t) => Ok(ChosenTechnique::Admitted(*t)),
            None => Ok(ChosenTechnique::RegionRecompute),
        },
    }
}

/// Whether `technique`'s emitted write addresses stored rows individually —
/// and therefore structurally needs a proven per-row [`RowIdentity::Key`],
/// never a [`RowIdentity::WholeRow`] fallback — to be a real option for a
/// cell. **Read-only classification, additive only**: consulted by the
/// `smelt-runtime` technique-preview builder
/// (`docs/specs/ui_model_diagnostics.md` §Semantics "Technique preview set")
/// to decide a *display-only* `NotApplicable` verdict for a technique this
/// cell did not admit; it is never consulted by [`resolve_cell_choice`] or
/// any other real-execution admission path, and adding it changes no
/// existing function's resolved output (`docs/specs/ui_model_diagnostics.md`
/// §Design "Why preview *every* technique…": "the wider preview set is
/// display-only … `resolve_cell_choice`'s real-execution semantics are
/// unchanged").
///
/// `Technique::DeleteInsert` (region recompute) addresses a whole partition
/// region, never an individual row, so it needs no row identity at all —
/// `false`. Every targeted-write technique (`KeyedFold`, `ColumnScopedMerge`,
/// `InPlaceUpdate`) addresses rows individually by key — `true`.
pub fn technique_requires_row_identity(technique: Technique) -> bool {
    match technique {
        Technique::DeleteInsert => false,
        Technique::KeyedFold
        | Technique::ColumnScopedMerge
        | Technique::InPlaceUpdate
        | Technique::PerGroupRecompute
        | Technique::SuccessionPatch => true,
    }
}

fn resolvable_set_label(
    admitted_technique: Option<&Technique>,
    backend_supports_column_scoped_merge: bool,
) -> String {
    let mut members = vec!["recompute".to_string()];
    if let Some(t) = admitted_technique {
        let live = match t {
            Technique::ColumnScopedMerge => backend_supports_column_scoped_merge,
            _ => true,
        };
        if live {
            members.push(format!("{t:?}"));
        }
    }
    members.join(", ")
}

/// Whether a `Technique::ColumnScopedMerge` cell's matched arm may write
/// conditionally (T1, `docs/plans/20260715-composed-axes-conditional-
/// maintenance.md` Phase C4) — the interchangeable alternative to always
/// rewriting every matched row: [`emit::emit_column_scoped_merge_suppressed`]
/// versus the unconditional [`emit::emit_column_scoped_merge`]
/// (`super::emit`). Both variants are members of the same resolvable
/// `ColumnScopedMerge` technique; this only decides which matched-arm shape
/// is safe to emit for one cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteSuppression {
    /// Every compared column is proven `Comparable` across runs (P3) over a
    /// proven, non-`WholeRow` row identity (P2) — the matched arm may be
    /// guarded by `IS DISTINCT FROM` over exactly these columns.
    Suppressed { compared_columns: Vec<String> },
    /// Fail-closed refusal of the conditional variant: at least one compared
    /// column is not proven comparable, no row identity is proven, or the
    /// group is empty. `why` names the specific column(s) or condition that
    /// refused, so a caller (`smelt explain`) can show the reason rather
    /// than only ever seeing the safe fallback.
    Unconditional { why: String },
}

/// Resolve whether a column-scoped `MERGE` cell's write may be suppressed
/// for unchanged rows (`incremental_models.md` §"Known Divergences" — this
/// phase narrows the "every emitted MERGE writes all matched rows
/// unconditionally" divergence).
///
/// Fail-closed over two independent proofs, either alone refusing:
/// - **P2, row identity** (`super::RowIdentityVerdict`, `derive::row_identity`):
///   a [`RowIdentity::WholeRow`] cell has no proven per-row join identity to
///   compare on safely, so suppression refuses regardless of column
///   comparability.
/// - **P3, per-column change comparability** (`crate::analysis::walk::
///   Comparability`, the property-walk fold): every column in `group_columns`
///   must carry a `Comparable` verdict in `comparability` — a column absent
///   from the vector is treated exactly like an explicit `Incomparable` verdict
///   (fail-closed: absence of a proof is never trusted as a pass), matching
///   `Comparability::default()`'s own fail-closed convention.
///
/// `group_columns` is the cell's own mutation-sensitive column group (the
/// caller resolves this from the same derived plan the cell came from —
/// e.g. `ColumnGroup::columns` matching `PlanCell::group`'s display name);
/// an empty group has nothing to compare and refuses.
pub fn resolve_write_suppression(
    group_columns: &[String],
    comparability: &[ColumnComparability],
    row_identity: &RowIdentityVerdict,
) -> WriteSuppression {
    if matches!(row_identity.identity, RowIdentity::WholeRow) {
        return WriteSuppression::Unconditional {
            why: "no proven row identity (P2 verdict is WholeRow) — a conditional write cannot \
                  safely address individual rows to compare, so the matched arm falls back to \
                  unconditional rewrite"
                .to_string(),
        };
    }

    if group_columns.is_empty() {
        return WriteSuppression::Unconditional {
            why: "the cell's column group is empty — there is nothing to compare".to_string(),
        };
    }

    let incomparable: Vec<String> = group_columns
        .iter()
        .filter(|col| {
            let verdict = comparability
                .iter()
                .find(|c| c.output.eq_ignore_ascii_case(col));
            match verdict {
                Some(c) => c.comparability == Comparability::Incomparable,
                // Fail-closed: no proof at all for this column is never
                // trusted as a pass.
                None => true,
            }
        })
        .cloned()
        .collect();

    if incomparable.is_empty() {
        WriteSuppression::Suppressed {
            compared_columns: group_columns.to_vec(),
        }
    } else {
        WriteSuppression::Unconditional {
            why: format!(
                "column(s) {} are not proven comparable across runs (P3) — the conditional \
                 write refuses fail-closed and falls back to the unconditional matched-arm \
                 rewrite",
                incomparable.join(", ")
            ),
        }
    }
}

/// Resolve whether a **keyless** (`RowIdentity::WholeRow`) region's
/// staged-candidate write may be suppressed (`docs/outcomes/
/// 20260815-definition-delta-migrate/phases/27c-plan.md`) — the whole-row
/// counterpart of [`resolve_write_suppression`], which refuses outright
/// whenever the row identity is `WholeRow`. Unlike the keyed proof, which
/// only needs a cell's own mutation-sensitive column group to be comparable
/// (the diff join addresses rows by key, so only the compared group's values
/// matter), a keyless diff is a whole-row `EXCEPT ALL` — every selected
/// column participates in row equality, so `output_columns` here is the
/// model's full payload column set, not one cell's own group.
///
/// Fail-closed, mirroring [`resolve_write_suppression`]'s own posture:
/// - A proven `RowIdentity::Key` never resolves here — that is the keyed
///   mechanism's proof (`resolve_write_suppression`), never this one's.
/// - An empty `output_columns` has nothing to compare and refuses.
/// - A column absent from `comparability` is treated exactly like an
///   explicit `Incomparable` verdict — absence of a proof is never trusted
///   as a pass.
pub fn resolve_keyless_staged_suppression(
    output_columns: &[String],
    comparability: &[ColumnComparability],
    row_identity: &RowIdentityVerdict,
) -> WriteSuppression {
    if !matches!(row_identity.identity, RowIdentity::WholeRow) {
        return WriteSuppression::Unconditional {
            why: "a proven row identity (P2 verdict is Key) routes through the keyed \
                  staged-candidate mechanism, never the keyless whole-row one"
                .to_string(),
        };
    }

    if output_columns.is_empty() {
        return WriteSuppression::Unconditional {
            why: "the model has no payload output columns to compare".to_string(),
        };
    }

    let incomparable: Vec<String> = output_columns
        .iter()
        .filter(|col| {
            let verdict = comparability
                .iter()
                .find(|c| c.output.eq_ignore_ascii_case(col));
            match verdict {
                Some(c) => c.comparability == Comparability::Incomparable,
                // Fail-closed: no proof at all for this column is never
                // trusted as a pass.
                None => true,
            }
        })
        .cloned()
        .collect();

    if incomparable.is_empty() {
        WriteSuppression::Suppressed {
            compared_columns: output_columns.to_vec(),
        }
    } else {
        WriteSuppression::Unconditional {
            why: format!(
                "column(s) {} are not proven comparable across runs (P3) — the keyless \
                 conditional write refuses fail-closed and falls back to the unconditional \
                 region rewrite",
                incomparable.join(", ")
            ),
        }
    }
}

/// Why a suppressible cell's [`WriteSuppression`] verdict is or isn't
/// **preferred** once admitted — the conditional-variant dimension the
/// override ladder ranks alongside family choice
/// (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase
/// G1; `incremental_models.md` §"Windowed maintenance and the horizon"
/// category 2; `docs/research/20260715-conditional-maintenance-without-cdf.md`
/// item 7: "suppression is pointless on first build … the plan should
/// admit-but-not-prefer it there"). `NotAdmitted` mirrors
/// [`WriteSuppression::Unconditional`]'s own proof failure (P2/P3) — there
/// is no preference question at all when the conditional variant was never
/// admitted in the first place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariantReason {
    /// The write-suppression proof itself refused (P2 `WholeRow` identity,
    /// an incomparable compared column, or an empty group) — never a
    /// preference question.
    NotAdmitted,
    /// Admitted, but the trigger has no prior stored state on this column
    /// group to diff against — the compare would be pure overhead, not a
    /// correctness gain, so the cell resolves the unconditional matched arm
    /// by default despite the conditional variant being provably safe.
    FirstBuildPosture,
    /// Admitted and preferred: a steady-state trigger over prior state
    /// defaults to the change-suppressed matched arm.
    SteadyStatePreference,
    /// An explicit `prefer`/`technique` override on this dimension
    /// (`TechniquePreference::Suppress`/`Unconditional`,
    /// `CellTechnique::Suppress`/`Unconditional`) decided this, bypassing
    /// the structural first-build/steady-state default above — the same
    /// override-ladder precedence [`resolve_cell_choice`] applies to family
    /// choice, folded into this orthogonal dimension.
    Overridden,
}

/// Whether `trigger`'s cell has any prior stored state on its compared
/// column group to diff against. [`Trigger::Backfill`] (an explicit
/// whole-region recompute — a first build with no target table yet routes
/// through it, since there is nothing stored at all) and a definition-change
/// cell's ledger catch-up (`PlanCell::ledger_catch_up` — the group's ledger
/// entries start at `S = ∅` over existing regions, per that field's own doc
/// comment) both have nothing to compare: every row's "prior" value for this
/// group is *absent*, not merely stale, so a suppression compare there buys
/// nothing. A steady-state [`Trigger::NewData`]/[`Trigger::UpstreamMutation`]
/// trigger over an already-populated group has committed prior state and
/// stands to gain from suppression.
///
/// Both inputs are already-derived `PlanCell` fields — this is a pure
/// function of data every caller already holds, never a fresh "is this the
/// first run" check bolted on at the call site.
pub fn trigger_has_prior_state(trigger: &Trigger, ledger_catch_up: bool) -> bool {
    !ledger_catch_up && !matches!(trigger, Trigger::Backfill)
}

/// Fold the first-build/definition-change-backfill posture into an
/// already-resolved [`WriteSuppression`] verdict: a cell that is admitted
/// but not preferred resolves to the unconditional matched arm by default —
/// bit-identical to a genuine proof failure — but the returned
/// [`VariantReason`] distinguishes the two so a caller (`smelt explain`) can
/// show *which* happened rather than only ever seeing the safe fallback.
///
/// This is the resolver-side rule the plan's Goal names ("the first-build/
/// backfill posture is a rule in the resolver, not a runtime special case
/// bolted on in `maintenance_driver.rs`"): `trigger` and `ledger_catch_up`
/// are both already-derived `PlanCell` fields, so no caller needs its own ad
/// hoc "is this the first run" check.
///
/// `overrides` folds the override ladder's write-suppression dimension in
/// (`EffectiveOverride`, already narrowed by [`effective_override`]) — the
/// same `defaults.prefer` → `cells[].prefer` → `cells[].technique` ladder
/// [`resolve_cell_choice`] applies to family choice, reused here for this
/// orthogonal dimension via the same struct's `prefer`/`technique` fields
/// and the `Suppress`/`Unconditional` values those enums carry:
/// - `overrides.technique == Some(CellTechnique::Suppress)` is a hard pin
///   forcing the suppressed variant on, bypassing the structural default —
///   but never bypassing the P2/P3 admission proof: when `suppression` is
///   already `Unconditional` (the proof itself refused), this refuses with
///   a [`ChoiceRefusal`], the same "a pin bypasses the cost model, never
///   admission" rule `resolve_cell_choice` applies to family pins.
/// - `overrides.technique == Some(CellTechnique::Unconditional)` is a hard
///   pin forcing the plain unconditional matched arm — always admissible,
///   so it never refuses.
/// - `overrides.prefer`'s `Suppress`/`Unconditional` values are the soft
///   equivalents: they nudge the structural default without refusing,
///   falling back silently when the preferred variant isn't admitted.
/// - Any other `overrides.technique`/`prefer` value (a family pin/bias, or
///   `Auto`/`None`) leaves this dimension to the structural first-build/
///   steady-state default below, exactly as before this phase.
pub fn resolve_write_variant(
    suppression: &WriteSuppression,
    trigger: &Trigger,
    ledger_catch_up: bool,
    overrides: &EffectiveOverride,
) -> Result<(WriteSuppression, VariantReason), ChoiceRefusal> {
    // A hard pin on this dimension is consulted first, before the soft
    // `prefer` bias and before the structural default — mirroring
    // `resolve_cell_choice`'s own pin-before-prefer-before-default order.
    match overrides.technique {
        Some(CellTechnique::Suppress) => {
            return match suppression {
                WriteSuppression::Suppressed { compared_columns } => Ok((
                    WriteSuppression::Suppressed {
                        compared_columns: compared_columns.clone(),
                    },
                    VariantReason::Overridden,
                )),
                WriteSuppression::Unconditional { why } => Err(ChoiceRefusal {
                    trigger: trigger_label(trigger),
                    pinned: PinnedRequest::Technique(CellTechnique::Suppress),
                    why: format!(
                        "`technique: suppress` pins the change-suppressed matched arm on for \
                         this cell, but the write-suppression proof itself refused ({why}) — a \
                         pin bypasses the cost model/structural default, never the P2/P3 \
                         admission proof"
                    ),
                }),
            };
        }
        Some(CellTechnique::Unconditional) => {
            return Ok((
                WriteSuppression::Unconditional {
                    why: "pinned via `technique: unconditional` — the matched arm always \
                          rewrites every matched row regardless of the write-suppression proof \
                          or trigger posture"
                        .to_string(),
                },
                VariantReason::Overridden,
            ));
        }
        // A family pin (`Fold`/`Recompute`/`RederiveColumns`), `Auto`, or no
        // pin at all — this dimension falls through to the soft `prefer`
        // bias, then the structural default.
        _ => {}
    }

    // No hard pin on this dimension: a soft `prefer` nudges the structural
    // default without ever refusing — falling back silently when the
    // preferred variant isn't admitted, the same "soft" contract
    // `resolve_cell_choice` documents for family choice.
    match overrides.prefer {
        Some(TechniquePreference::Suppress) => {
            if let WriteSuppression::Suppressed { compared_columns } = suppression {
                return Ok((
                    WriteSuppression::Suppressed {
                        compared_columns: compared_columns.clone(),
                    },
                    VariantReason::Overridden,
                ));
            }
        }
        Some(TechniquePreference::Unconditional) => {
            return Ok((
                WriteSuppression::Unconditional {
                    why: "soft-preferred via `prefer: unconditional`, overriding the \
                          steady-state default"
                        .to_string(),
                },
                VariantReason::Overridden,
            ));
        }
        // A family bias (`Fold`/`Recompute`), `Auto`, or no bias at all —
        // falls through to the structural default.
        _ => {}
    }

    // No override on this dimension: the structural first-build/
    // definition-change-backfill posture default.
    Ok(match suppression {
        WriteSuppression::Unconditional { why } => (
            WriteSuppression::Unconditional { why: why.clone() },
            VariantReason::NotAdmitted,
        ),
        WriteSuppression::Suppressed { compared_columns } => {
            if trigger_has_prior_state(trigger, ledger_catch_up) {
                (
                    WriteSuppression::Suppressed {
                        compared_columns: compared_columns.clone(),
                    },
                    VariantReason::SteadyStatePreference,
                )
            } else {
                (
                    WriteSuppression::Unconditional {
                        why: "admitted (P2 row identity and P3 column comparability both \
                              proven) but not preferred: this trigger has no prior stored \
                              state on the compared column group to diff against (first \
                              build or a definition-change backfill) — the compare would be \
                              pure overhead, not a correctness gain, so the cell resolves \
                              the unconditional matched arm by default"
                            .to_string(),
                    },
                    VariantReason::FirstBuildPosture,
                )
            }
        }
    })
}

/// Fold the write-suppression proof and its variant resolution into one call
/// — `model_property_vector(sql, ...).comparability` →
/// [`resolve_write_suppression`] → [`resolve_write_variant`] — the exact
/// sequence `maintenance_driver.rs`'s live `ColumnScopedMerge`/`KeyedFold`
/// resolvers each already run inline, and the one a preview builder
/// (`smelt explain --show-sql`) must run identically so a printed statement
/// can never drift from what a live run would execute
/// (`incremental_models.md` §"Statement emission (single owner)"). Drops the
/// `VariantReason` a caller that only wants the resolved SQL shape has no use
/// for — a caller that also wants to explain *why* (`smelt explain`'s report
/// line) still calls [`resolve_write_suppression`]/[`resolve_write_variant`]
/// directly, as `smelt-cli/src/explain.rs` already does.
pub fn resolve_cell_write_suppression(
    sql: &str,
    group_columns: &[String],
    cell: &PlanCell,
    overrides: &EffectiveOverride,
) -> Result<WriteSuppression, ChoiceRefusal> {
    // join-context: no-context-field (reads only `.comparability` below, no
    // context-dependent field of the vector)
    let comparability = crate::analysis::walk::model_property_vector(
        sql,
        &crate::analysis::join_shape::JoinContext::new(),
    )
    .map(|v| v.comparability)
    .unwrap_or_default();
    let raw_suppression =
        resolve_write_suppression(group_columns, &comparability, &cell.row_identity);
    resolve_write_variant(
        &raw_suppression,
        &cell.trigger,
        cell.ledger_catch_up,
        overrides,
    )
    .map(|(suppression, _reason)| suppression)
}

/// Whether a region `DeleteInsert` recompute may realise the change-
/// suppressed staged write (`emit::emit_diff_patch`, `slice_predicate =
/// region.predicate(...)`, `DeleteLeg::Complete`) instead of unconditionally
/// rewriting its whole window — the region family's own conditional
/// variant, mirroring [`WriteSuppression`] for the column-scoped `MERGE`
/// family (`docs/specs/model_transforms.md` §"Change-suppressed MERGE and
/// the staged-candidate conditional DELETE+INSERT"). `key` carries the
/// proven row identity `emit_diff_patch`'s diff join addresses rows by —
/// `WriteSuppression` has no use for it (a `MERGE`'s `unique_key` already
/// comes from elsewhere), but the staged-candidate emitter needs it
/// threaded through explicitly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegionWrite {
    /// The plain unconditional region rewrite — today's byte-identical
    /// `emit::emit_delete_insert` widened scan.
    Unconditional { why: String },
    /// The change-suppressed staged write is admitted and preferred: a
    /// proven row identity plus a fully comparable column group license
    /// `emit::emit_diff_patch` over the region's own slice predicate.
    Suppressed {
        key: Vec<String>,
        compared_columns: Vec<String>,
    },
}

/// Resolve a region `DeleteInsert` recompute's write variant — composed
/// entirely from [`resolve_write_suppression`] (the same P2/P3 proof the
/// column-scoped `MERGE` family already reuses) and [`resolve_write_variant`]
/// (the same first-build/steady-state posture and override ladder), never a
/// second proof. `row_identity`'s `RowIdentity::Key` is threaded into the
/// `Suppressed` verdict for [`emit::emit_diff_patch`]'s diff-join key —
/// `resolve_write_suppression` already refuses outright (`Unconditional`)
/// whenever `row_identity` is `WholeRow`, so a `Suppressed` result here is
/// only ever reached with a proven key.
pub fn resolve_region_write_variant(
    group_columns: &[String],
    comparability: &[ColumnComparability],
    row_identity: &RowIdentityVerdict,
    trigger: &Trigger,
    ledger_catch_up: bool,
    overrides: &EffectiveOverride,
) -> Result<RegionWrite, ChoiceRefusal> {
    let suppression = resolve_write_suppression(group_columns, comparability, row_identity);
    let (resolved, _reason) =
        resolve_write_variant(&suppression, trigger, ledger_catch_up, overrides)?;
    Ok(match resolved {
        WriteSuppression::Unconditional { why } => RegionWrite::Unconditional { why },
        WriteSuppression::Suppressed { compared_columns } => {
            let key = match &row_identity.identity {
                RowIdentity::Key(key) => key.clone(),
                RowIdentity::WholeRow => {
                    unreachable!(
                        "resolve_write_suppression refuses (Unconditional) whenever \
                         row_identity is WholeRow — a Suppressed verdict is only ever \
                         reached with a proven key"
                    )
                }
            };
            RegionWrite::Suppressed {
                key,
                compared_columns,
            }
        }
    })
}

/// Which physical write mechanism realizes a keyed-fold cell's conditional
/// write (T1/T2, `docs/plans/20260715-composed-axes-conditional-
/// maintenance.md` Phase C5): the ordinary keyed `MERGE`
/// ([`super::emit::emit_keyed_fold`]/[`super::emit::
/// emit_keyed_fold_suppressed`]) on a backend that can run `MERGE` at all,
/// else the merge-less **staged-candidate conditional DELETE+INSERT**
/// ([`super::emit::emit_staged_candidate_conditional`]) — the keyed-shaped
/// realisation for a backend without `MERGE` (a documented gap:
/// Spark-over-Parquet). `MERGE` is preferred whenever the backend has it;
/// the staged-candidate mechanism is never a silent substitute on a backend
/// that *can* run `MERGE` (`docs/specs/model_transforms.md` §"Change-
/// suppressed MERGE and the staged-candidate conditional DELETE+INSERT").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyedWriteMechanism {
    /// The keyed `MERGE`, carrying its own resolved [`WriteSuppression`]
    /// verdict (suppressed or the plain unconditional matched arm).
    Merge(WriteSuppression),
    /// The merge-less staged-candidate conditional write, over exactly
    /// `compared_columns` — only ever produced when [`resolve_write_suppression`]
    /// proved the group fully comparable over a proven row identity (this
    /// phase's staged-candidate emitter has no unconditional shape, so an
    /// `Unconditional` verdict on a `MERGE`-less backend cannot resolve to
    /// this mechanism — see [`resolve_keyed_write_mechanism`]'s doc comment
    /// for the fallback that case requires from the caller).
    StagedCandidate { compared_columns: Vec<String> },
}

/// Resolve which mechanism realizes a keyed-fold cell's write, given its
/// already-resolved [`WriteSuppression`] verdict
/// ([`resolve_write_suppression`]) and whether the target backend can run
/// `MERGE` at all.
///
/// `None` means neither mechanism this function knows about is admissible:
/// the backend cannot run `MERGE`, and the compare group was not fully
/// comparable (or no row identity was proven) — `WriteSuppression::
/// Unconditional`. There is no merge-less *unconditional* keyed-fold emitter
/// in this catalogue (the staged-candidate shape's `DELETE`+`INSERT` only
/// makes sense restricted to the rows whose effect is not the identity —
/// see [`super::emit::emit_staged_candidate_conditional`]'s panic contract);
/// a caller reaching `None` must fall back to a backend-agnostic mechanism
/// outside this function's scope (e.g. the always-available whole-region
/// recompute, [`ChosenTechnique::RegionRecompute`]), never invent a
/// merge-less unconditional MERGE substitute.
///
/// Consults an optional `write:` pin (`incremental_models.md` §"Per-cell
/// write addressing" → "User pins"): within the `KeyedFold` technique
/// family, `keyed`/`keyed_conditional` and `staged_candidate` all select the
/// same *technique* but pin different **mechanisms**. `keyed`/
/// `keyed_conditional` pin the `MERGE` mechanism — refusing
/// (`ChoiceRefusal`) rather than silently falling back to the
/// staged-candidate shape when the backend can't run `MERGE` (a fail-closed
/// second line of defence behind the open registry's own
/// `WriteCapability::Merge` check, `resolve_write_pin`). `staged_candidate`
/// pins the merge-less staged conditional `DELETE`+`INSERT` **even on a
/// `MERGE`-capable backend** — an explicit pin is not a downgrade to be
/// second-guessed — and refuses when `suppression` is `Unconditional`: the
/// staged-candidate emitter has no unconditional shape, so there is nothing
/// to fall back to except a silent substitution, which a pin must never
/// produce. A pin naming any other pattern (e.g. `region`) is outside this
/// function's family — [`resolve_cell_choice`] itself already resolved (or
/// refused) that case, so this function does not second-guess it and just
/// applies the unpinned default below.
///
/// Absent a pin (or given one outside the `KeyedFold` family),
/// `Ok(None)`/`Ok(Some(..))` matches the unpinned default: `MERGE` is
/// preferred whenever the backend has it, and the staged-candidate
/// mechanism is reachable only through a genuine capability gap
/// (`backend_supports_merge = false` with a `Suppressed` verdict) — never a
/// preference. `Ok(None)` means neither mechanism this function knows about
/// is admissible: the backend cannot run `MERGE`, and the compare group was
/// not fully comparable (or no row identity was proven) —
/// `WriteSuppression::Unconditional`. There is no merge-less *unconditional*
/// keyed-fold emitter in this catalogue (the staged-candidate shape's
/// `DELETE`+`INSERT` only makes sense restricted to the rows whose effect is
/// not the identity — see [`super::emit::emit_staged_candidate_conditional`]'s
/// panic contract); a caller reaching `Ok(None)` must fall back to a
/// backend-agnostic mechanism outside this function's scope (e.g. the
/// always-available whole-region recompute,
/// [`ChosenTechnique::RegionRecompute`]), never invent a merge-less
/// unconditional MERGE substitute.
pub fn resolve_keyed_write_mechanism(
    suppression: &WriteSuppression,
    backend_supports_merge: bool,
    write_pin: Option<&'static super::WritePattern>,
) -> Result<Option<KeyedWriteMechanism>, ChoiceRefusal> {
    if let Some(pattern) = write_pin {
        match pattern.name {
            "staged_candidate" => {
                return match suppression {
                    WriteSuppression::Suppressed { compared_columns } => {
                        Ok(Some(KeyedWriteMechanism::StagedCandidate {
                            compared_columns: compared_columns.clone(),
                        }))
                    }
                    WriteSuppression::Unconditional { why } => Err(ChoiceRefusal {
                        trigger: "keyed-fold cell".to_string(),
                        pinned: PinnedRequest::Write(pattern.name.to_string()),
                        why: format!("write: staged_candidate has no unconditional form — {why}"),
                    }),
                };
            }
            "keyed" | "keyed_conditional" => {
                return if backend_supports_merge {
                    Ok(Some(KeyedWriteMechanism::Merge(suppression.clone())))
                } else {
                    Err(ChoiceRefusal {
                        trigger: "keyed-fold cell".to_string(),
                        pinned: PinnedRequest::Write(pattern.name.to_string()),
                        why: "the backend cannot run MERGE — write: keyed/keyed_conditional \
                              pins the MERGE mechanism, never a silent substitute"
                            .to_string(),
                    })
                };
            }
            _ => {}
        }
    }
    Ok(default_keyed_write_mechanism(
        suppression,
        backend_supports_merge,
    ))
}

fn default_keyed_write_mechanism(
    suppression: &WriteSuppression,
    backend_supports_merge: bool,
) -> Option<KeyedWriteMechanism> {
    if backend_supports_merge {
        return Some(KeyedWriteMechanism::Merge(suppression.clone()));
    }
    match suppression {
        WriteSuppression::Suppressed { compared_columns } => {
            Some(KeyedWriteMechanism::StagedCandidate {
                compared_columns: compared_columns.clone(),
            })
        }
        WriteSuppression::Unconditional { .. } => None,
    }
}

/// Whether a cell's recompute may restrict its scan to an exact upstream
/// delta's changed-key set (T3, `docs/specs/model_transforms.md` §"Delta-
/// restricted enrichment join"). Built for a model-edge creation cell's
/// region recompute (`Technique::DeleteInsert`, the `RecomputeRegion`
/// corner — `docs/plans/20260715-composed-axes-conditional-maintenance.md`
/// Phase E3) and reused unchanged for an `UpstreamMutation` cell's
/// column-scoped-MERGE enrichment recompute driven by an external
/// `mutation_profile: mutable_snapshot` source, whose exact delta is the
/// fingerprint sidecar's synthesized changed-key set instead of a model
/// edge's recorded observed delta (T3 over external sources, Phase F5) —
/// this function's admission logic does not distinguish the two: it only
/// ever consults a [`super::SkeletonSourceClosure`] verdict and an exact
/// delta key set, both already provider-agnostic by construction, so no
/// change was needed to extend the licence (the "licence union" the phase
/// wires — `derive::mutation_enrichment_closure` is the analogous closure
/// derivation for the external-source case, mirroring `derive::
/// model_edge_enrichment_closure`'s already-landed one for model edges).
/// Licensed only by the conjunction of two independent facts — either
/// alone falls back to the ordinary widened scan, never a partial
/// restriction:
/// - **P1, skeleton-source closure** (`super::SkeletonSourceClosure`,
///   `crate::analysis::skeleton_closure`): every enrichment join in the
///   cell's model must be proven `Closed`, so the driving edge's changed
///   keys are provably the *only* rows whose output can have changed.
/// - **An exact, non-empty observed delta** on the driving edge for this
///   cell's run window (Group D, T5's recorded delta) — an absent delta
///   (pre-D2 upstream, or a window never recorded) and a present-but-empty
///   delta (nothing changed — the payoff belongs to the empty-delta no-op
///   cascade, a later phase's scope, not this restriction) both fall back
///   to the widened scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecomputeRestriction {
    /// Both factors hold: restrict the region recompute to `delta_keys` via
    /// [`super::emit::emit_delete_insert_delta_restricted`]'s semi-join.
    Restricted { delta_keys: Vec<String> },
    /// Either factor is absent — run the ordinary widened scan
    /// ([`super::emit::emit_delete_insert`]), byte-identical to today's
    /// unrestricted form. `why` names which factor was missing (never
    /// silently indistinguishable from the restricted case).
    Unrestricted { why: String },
}

/// Resolve [`RecomputeRestriction`] for one model-edge creation cell.
///
/// `skeleton_source_closure` is the cell's own `PlanCell::
/// skeleton_source_closure` verdict — `None` (no enrichment join to close
/// over) is treated exactly like `Some(Open { .. })`: restriction needs a
/// *proven* closed skeleton, not the mere absence of an enrichment join
/// fact. `observed_delta` is the driving edge's recorded changed-key set for
/// this run's window, `None` distinguishing "never recorded" from
/// `Some(&[])`'s "recorded and empty" (`incremental_models.md` §"The graph
/// layer" — "Empty and absent are distinct").
pub fn resolve_recompute_restriction(
    skeleton_source_closure: Option<&super::SkeletonSourceClosure>,
    observed_delta: Option<&[String]>,
) -> RecomputeRestriction {
    let closed = skeleton_source_closure.is_some_and(|c| c.is_closed());
    if !closed {
        return RecomputeRestriction::Unrestricted {
            why: "skeleton-source closure (P1) is not proven Closed for this cell's enrichment \
                  join(s) — the delta-restricted recompute is not licensed"
                .to_string(),
        };
    }
    match observed_delta {
        Some(keys) if !keys.is_empty() => RecomputeRestriction::Restricted {
            delta_keys: keys.to_vec(),
        },
        Some(_) => RecomputeRestriction::Unrestricted {
            why: "the recorded observed delta for this window is empty — nothing changed \
                  upstream, so there is no key set to restrict to"
                .to_string(),
        },
        None => RecomputeRestriction::Unrestricted {
            why: "no observed delta is recorded for this window — the ordinary widened scan \
                  is the fail-closed default (widen-never-narrow)"
                .to_string(),
        },
    }
}

/// The column a [`RecomputeRestriction::Restricted`] semi-join predicates
/// on for an `UpstreamMutation` cell driven by an external source (T3 over
/// external sources, `docs/plans/20260715-composed-axes-conditional-
/// maintenance.md` Phase F5) — as opposed to a model-edge creation cell,
/// where the restriction column is the OUTPUT's own row-identity key
/// (`maintenance_driver::DeltaRestrictionFacts::restrict_column`, since a
/// model edge's delta flows straight through in the same key domain). An
/// external source's fingerprint-sidecar-derived delta is keyed by the
/// SOURCE's own row identity instead — the dimension's declared
/// `unique_key` — which the enrichment join's equality condition equates
/// against the driving (fact) side's own column of the SAME name (the
/// common same-name foreign-key convention this v1 restriction assumes;
/// `docs/specs/sources.md` §"Row identity"). `None` for a composite or
/// undeclared unique key — this v1 restriction, like the model-edge one,
/// is single-column only; the caller's safe default is the ordinary
/// widened scan.
pub fn enrichment_restrict_column(dimension_unique_key: &[String]) -> Option<&str> {
    match dimension_unique_key {
        [only] => Some(only.as_str()),
        _ => None,
    }
}

#[cfg(test)]
mod enrichment_restrict_column_tests;
#[cfg(test)]
mod keyed_write_mechanism_tests;
#[cfg(test)]
mod keyless_write_suppression_tests;
#[cfg(test)]
mod recompute_restriction_tests;
#[cfg(test)]
mod region_write_variant_tests;
#[cfg(test)]
mod technique_requires_row_identity_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod write_suppression_tests;
#[cfg(test)]
mod write_variant_tests;
