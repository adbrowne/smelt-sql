//! `derive_backbuild_options`: turn a [`super::DefinitionDiff`] plus
//! [`super::BackbuildInputs`] into a [`super::BackbuildOptions`] value, and
//! `assemble`: turn a [`super::BackbuildOptions`] plus a [`Selection`] into
//! an ordered statement script. See the module doc comment in
//! `backbuild/mod.rs` and `docs/research/20260802-backbuild-synthesis.md`
//! §2 ("The contract"), §4 ("The catalogue" — G-class "Honest refusals" and
//! the CTE posture note), and §6 ("Architecture").
//!
//! This phase implements exactly the refusal paths research §4's G-class
//! names outright — G1 (grain change), G2 (join-multiplicity change), and a
//! changed CTE (or any other whole-definition opacity) — plus the A0 no-op
//! short-circuit and the always-present model-level `FullRefresh` baseline.
//! No admissible targeted technique (B1/B2/D1/…) is produced here; that
//! starts with later phases. Every diff shape this phase does not
//! recognise still yields a named, fail-closed refusal rather than a
//! silently empty atom list — fail-loud discipline
//! (`docs/specs/architecture.md` §"Fail-loud discipline").

use super::{
    AtomAnalysis, AtomicChange, BackbuildInputs, BackbuildOption, BackbuildOptions,
    BackbuildRefusal, DefinitionDiff, HSlot, SkeletonDiff, Technique, WriteScope,
};

/// Derive the option set for a `(before, after)` diff (research §2, §4).
pub fn derive_backbuild_options(
    diff: &DefinitionDiff,
    inputs: &BackbuildInputs,
) -> BackbuildOptions {
    let full_refresh = full_refresh_option(inputs);

    let atoms = match diff {
        DefinitionDiff::Opaque { reason } => vec![whole_definition_refusal(reason)],
        DefinitionDiff::Comparable(comparable) => {
            if diff.is_noop() {
                // A0 — no-op: nothing changed, so there is nothing to
                // refuse or classify. The targeted script is trivially
                // empty (`assemble` over zero atoms); `FullRefresh` remains
                // available regardless.
                Vec::new()
            } else if let SkeletonDiff::Changed { reason } = &comparable.skeleton {
                vec![skeleton_refusal(reason)]
            } else {
                // Something changed (select list, WHERE, or set-ops) but
                // this phase does not yet classify it into an admissible
                // technique. A conservative, honest catch-all — never a
                // silently empty atom list for a definition that did
                // change.
                vec![unclassified_refusal()]
            }
        }
    };

    BackbuildOptions {
        atoms,
        full_refresh,
    }
}

fn full_refresh_option(inputs: &BackbuildInputs) -> BackbuildOption {
    BackbuildOption {
        technique: Technique::FullRefresh,
        slot: None,
        statements: vec![format!(
            "CREATE OR REPLACE TABLE {} AS {}",
            inputs.table, inputs.after_sql
        )],
        write_scope: WriteScope::FullWrite,
        reads_upstream: true,
        // CREATE OR REPLACE TABLE ... AS <after> re-evaluated against the
        // same inputs reproduces the same result every time.
        rerun_safe: true,
    }
}

fn whole_definition_refusal(reason: &str) -> AtomAnalysis {
    AtomAnalysis {
        change: AtomicChange::WholeDefinition {
            reason: reason.to_string(),
        },
        options: Vec::new(),
        inadmissible: vec![BackbuildRefusal {
            atom: "whole-definition".to_string(),
            reason: format!(
                "the definition diff could not be factored, so no targeted technique can be \
                 proven: {reason}"
            ),
        }],
    }
}

fn skeleton_refusal(reason: &str) -> AtomAnalysis {
    AtomAnalysis {
        change: AtomicChange::Skeleton {
            reason: reason.to_string(),
        },
        options: Vec::new(),
        inadmissible: vec![BackbuildRefusal {
            atom: "skeleton".to_string(),
            reason: classify_skeleton_reason(reason),
        }],
    }
}

/// Label a `SkeletonDiff::Changed` reason with its research §4 catalogue
/// case where the diff module's reason text identifies one: G1 (grain
/// change — `GROUP BY`/`DISTINCT`) or G2 (join-multiplicity change). Any
/// other skeleton change (FROM target, `HAVING`/`QUALIFY`/`WINDOW`/`ORDER
/// BY`/`LIMIT`) is still refused, just without a G1/G2 label — the
/// catalogue's G-class explicitly refuses those too ("Opaque expressions
/// ... LIMIT/ORDER BY changes: refuse with named reasons").
fn classify_skeleton_reason(reason: &str) -> String {
    let lower = reason.to_lowercase();
    if lower.contains("group by") || lower.contains("distinct") {
        format!("G1 (grain change) — {reason}")
    } else if lower.contains("join") {
        format!("G2 (join-multiplicity change) — {reason}")
    } else {
        format!("skeleton changed, not yet admissible — {reason}")
    }
}

fn unclassified_refusal() -> AtomAnalysis {
    AtomAnalysis {
        change: AtomicChange::Unclassified,
        options: Vec::new(),
        inadmissible: vec![BackbuildRefusal {
            atom: "whole-definition".to_string(),
            reason: "the definition changed in a way this phase does not yet classify into an \
                     admissible technique (targeted-technique admission arrives in later phases)"
                .to_string(),
        }],
    }
}

/// One chosen technique per atom, or the always-present `FullRefresh`
/// baseline — the input `assemble` needs to turn a [`BackbuildOptions`]
/// value into an ordered statement script (research §6: "assemble(options,
/// selection) applies the H ordering to one chosen option per atom").
/// Choosing *among* an atom's options is a future cost model's job
/// (research §2 "Options, not choices"); this phase's callers supply the
/// choice directly.
#[derive(Debug, Clone)]
pub enum Selection {
    /// Compose the targeted script: for every atom in
    /// `BackbuildOptions::atoms`, in order, the index into that atom's
    /// `options` to use. Must have the same length as `atoms`; if any
    /// chosen index does not name an admissible option — including an atom
    /// whose `options` is empty — `assemble` returns an empty script.
    /// Partial application is never offered (research §2 "Refusal
    /// posture").
    Targeted { atom_choices: Vec<usize> },
    /// The always-present model-level `FullRefresh` baseline.
    FullRefresh,
}

/// Turn a [`BackbuildOptions`] value plus a [`Selection`] into an ordered
/// list of statement strings, ready to execute in order. Statements are
/// never authored here — every string comes from a `BackbuildOption`
/// classification already produced (statement single-ownership,
/// `docs/specs/architecture.md` §"Constraints & Invariants" item 12).
///
/// This phase's classifier only ever produces atoms with empty option
/// sets, so [`Selection::Targeted`] composes to an empty script in every
/// case it can reach today: zero atoms (the A0 no-op case), or one or more
/// atoms that all lack an admissible option (every refusal case this phase
/// derives). The H-ordering slot structure (research §4 "H. Composites":
/// `renames → ALTER ADD/TYPE → DELETEs → UPDATEs/MERGEs → INSERTs → ALTER
/// DROPs`) is built out in full regardless, so later phases — which
/// populate atom options with real techniques — only need to give each new
/// [`Technique`] an [`HSlot`]; this function's bucketing loop does not
/// change.
pub fn assemble(options: &BackbuildOptions, selection: &Selection) -> Vec<String> {
    match selection {
        Selection::FullRefresh => options.full_refresh.statements.clone(),
        Selection::Targeted { atom_choices } => {
            if atom_choices.len() != options.atoms.len() {
                return Vec::new();
            }

            // Pass 1: every atom must name an admissible option, or the
            // whole composition is refused — partial application is never
            // offered (research §2).
            let mut chosen = Vec::with_capacity(options.atoms.len());
            for (atom, &choice) in options.atoms.iter().zip(atom_choices) {
                match atom.options.get(choice) {
                    Some(opt) => chosen.push(opt),
                    None => return Vec::new(),
                }
            }

            // Pass 2: bucket into the H-ordering slots, then concatenate in
            // slot order.
            let mut renames = Vec::new();
            let mut alters = Vec::new();
            let mut deletes = Vec::new();
            let mut update_merges = Vec::new();
            let mut inserts = Vec::new();
            let mut drops = Vec::new();
            for opt in chosen {
                let Some(slot) = opt.slot else {
                    // A model-level-only technique (FullRefresh) ended up
                    // in an atom's option set — a classifier bug, not a
                    // valid targeted composition. Refuse rather than emit
                    // a mis-ordered script.
                    return Vec::new();
                };
                let bucket = match slot {
                    HSlot::Rename => &mut renames,
                    HSlot::Alter => &mut alters,
                    HSlot::Delete => &mut deletes,
                    HSlot::UpdateMerge => &mut update_merges,
                    HSlot::Insert => &mut inserts,
                    HSlot::Drop => &mut drops,
                };
                bucket.extend(opt.statements.iter().cloned());
            }

            renames
                .into_iter()
                .chain(alters)
                .chain(deletes)
                .chain(update_merges)
                .chain(inserts)
                .chain(drops)
                .collect()
        }
    }
}
