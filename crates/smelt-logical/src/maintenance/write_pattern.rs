use super::*;

use crate::analysis::walk::ColumnComparability;

// -------------------------------------------------------------------------
// Open write-pattern registry (`incremental_models.md` §"Per-cell write
// addressing", §"The write-pattern set is open").
// -------------------------------------------------------------------------

/// A contract fact the output must declare for a write pattern to be a
/// structural candidate — the first factor of the available-addressings
/// rule (`incremental_models.md` §"The available-addressings rule": "What
/// each declared fact gates").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractFact {
    /// A declared `unique_key` (row identity) — gates keyed `MERGE`,
    /// column-scoped `MERGE`, in-place `UPDATE`.
    Identity,
    /// A declared partition axis (`timeseries:`) to delete/scope by — gates
    /// region `DELETE`+`INSERT`.
    PartitionAxis,
}

/// The backend capability a pattern needs to execute at all — the fourth
/// admission factor (`incremental_models.md` §"The pattern set is
/// backend-relative"). `Always` patterns need nothing beyond the structural
/// facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteCapability {
    /// No backend-specific capability required.
    Always,
    /// Requires a keyed `MERGE` primitive.
    Merge,
    /// Requires a column-scoped `MERGE` (a target-row-preserving `MERGE …
    /// UPDATE SET *` over a full-row source projection).
    ColumnScopedMerge,
}

/// One entry of the open write-pattern registry: the name a `write:` pin
/// resolves against, the contract facts it structurally requires, and the
/// backend capability that gates it. New patterns plug in by adding an
/// entry here — the admission rule and equivalence gate, not the
/// enumeration, are the durable contract (`incremental_models.md` §"The
/// write-pattern set is open").
#[derive(Debug, Clone, Copy)]
pub struct WritePattern {
    pub name: &'static str,
    pub required_facts: &'static [ContractFact],
    pub capability: WriteCapability,
}

/// Which physical resolution a registry pattern maps to when it is
/// consulted as a `cells[].write` pin by [`choice::resolve_cell_choice`]
/// (`incremental_models.md` §"Per-cell write addressing" → "User pins":
/// the pin constrains technique selection, it does not merely validate and
/// get discarded). This is the single place the open registry's name space
/// meets the closed `Technique` enum a plan cell actually carries — new
/// registry entries must extend [`WritePattern::selects`], which is
/// exhaustive over `WRITE_PATTERN_REGISTRY`'s names (see that fn's `_ =>`
/// arm).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteSelection {
    /// `region`/`full_rebuild` both select the always-available whole-region
    /// recompute — the `ChosenTechnique::RegionRecompute` corner.
    RegionRecompute,
    /// A targeted pattern selects the plan cell's own admitted [`Technique`]
    /// of that family. `keyed_conditional`/`staged_candidate` both select
    /// [`Technique::KeyedFold`]: they are alternate write *mechanisms*
    /// within that technique family
    /// ([`choice::KeyedWriteMechanism`]), not distinct plan techniques, so
    /// pinning either only constrains which `Technique` variant the cell
    /// must have admitted — the mechanism-within-the-technique choice stays
    /// [`choice::resolve_keyed_write_mechanism`]'s job.
    Technique(Technique),
    /// `diff_patch` selects the diff-then-patch write pattern
    /// (`incremental_models.md` §"The write-pattern set is open"): compute
    /// the candidate slice, diff against stored state, write only the
    /// difference. Its own admission ([`diff_patch::admit_diff_patch`]) is a
    /// separate proof from any `Technique`'s admission — a selection-name
    /// marker, no payload, matching `RegionRecompute`'s own shape.
    DiffPatch,
}

impl WritePattern {
    /// The [`WriteSelection`] this pattern maps to. Exhaustive over
    /// [`WRITE_PATTERN_REGISTRY`]'s names — a name reachable through
    /// [`lookup_write_pattern`] with no arm here is a registry bug, not a
    /// runtime input to handle gracefully, hence the `unreachable!`.
    pub fn selects(&self) -> WriteSelection {
        match self.name {
            "region" | "full_rebuild" => WriteSelection::RegionRecompute,
            "keyed" | "keyed_conditional" | "staged_candidate" => {
                WriteSelection::Technique(Technique::KeyedFold)
            }
            "column" => WriteSelection::Technique(Technique::ColumnScopedMerge),
            "update" => WriteSelection::Technique(Technique::InPlaceUpdate),
            "diff_patch" => WriteSelection::DiffPatch,
            other => unreachable!(
                "write-pattern registry entry '{other}' has no WriteSelection mapping — extend \
                 WritePattern::selects when adding a new registry entry"
            ),
        }
    }
}

/// The currently-known write-pattern set (`incremental_models.md` §"Per-cell
/// write addressing"): region rewrite, keyed/column-scoped/in-place targeted
/// writes, full rebuild, and the two conditional (change-suppressed)
/// mechanisms C4/C5 introduced — a `MERGE`-capable keyed conditional write
/// and the merge-less staged-candidate conditional `DELETE`+`INSERT` for a
/// backend without `MERGE` (`docs/specs/model_transforms.md` §"Change-
/// suppressed MERGE and the staged-candidate conditional DELETE+INSERT"). A
/// `write:` pin resolves by name against this table; an unrecognised name,
/// or one the target backend cannot provide, is
/// `MaintenanceWritePatternUnavailable` ([`resolve_write_pin`]).
pub const WRITE_PATTERN_REGISTRY: &[WritePattern] = &[
    WritePattern {
        name: "region",
        required_facts: &[ContractFact::PartitionAxis],
        capability: WriteCapability::Always,
    },
    WritePattern {
        name: "keyed",
        required_facts: &[ContractFact::Identity],
        capability: WriteCapability::Merge,
    },
    WritePattern {
        name: "column",
        required_facts: &[ContractFact::Identity],
        capability: WriteCapability::ColumnScopedMerge,
    },
    WritePattern {
        name: "update",
        required_facts: &[ContractFact::Identity],
        capability: WriteCapability::Merge,
    },
    WritePattern {
        name: "full_rebuild",
        required_facts: &[],
        capability: WriteCapability::Always,
    },
    WritePattern {
        name: "keyed_conditional",
        required_facts: &[ContractFact::Identity],
        capability: WriteCapability::Merge,
    },
    WritePattern {
        name: "staged_candidate",
        required_facts: &[ContractFact::Identity],
        capability: WriteCapability::Always,
    },
    WritePattern {
        name: "diff_patch",
        required_facts: &[ContractFact::Identity],
        capability: WriteCapability::Always,
    },
];

/// Look up a registry entry by its open name (`maintenance.cells[].write`'s
/// resolution target). `None` for an unrecognised name.
pub fn lookup_write_pattern(name: &str) -> Option<&'static WritePattern> {
    WRITE_PATTERN_REGISTRY.iter().find(|p| p.name == name)
}

/// The output's declared contract facts a cell may draw on for the
/// available-addressings rule — plain data the caller (`smelt-db`,
/// `smelt-runtime`) already holds from the model's declared shape
/// (`ModelMetadata`); this module never re-derives it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OutputContractFacts {
    pub has_identity: bool,
    pub has_partition_axis: bool,
}

impl OutputContractFacts {
    fn satisfies(&self, fact: ContractFact) -> bool {
        match fact {
            ContractFact::Identity => self.has_identity,
            ContractFact::PartitionAxis => self.has_partition_axis,
        }
    }
}

/// Backend write-pattern capability, consulted as the fourth admission
/// factor (`incremental_models.md` §"The pattern set is backend-relative").
/// Derived from `smelt_dialect::BackendCapabilities` by the caller — this
/// module only consumes the two booleans it needs, never a concrete backend
/// type, staying below `smelt-backend`/`smelt-dialect` in the layering
/// (`CLAUDE.md` §"Layered single-ownership").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BackendWriteCapabilities {
    pub supports_merge: bool,
    pub supports_column_scoped_merge: bool,
}

impl BackendWriteCapabilities {
    /// A backend that can run every registered pattern — used where no
    /// concrete target backend is known yet (e.g. `smelt explain` with no
    /// declared target), so the admissible set reports the structural
    /// answer rather than under-reporting against an assumed backend.
    pub fn all() -> Self {
        BackendWriteCapabilities {
            supports_merge: true,
            supports_column_scoped_merge: true,
        }
    }

    fn provides(&self, capability: WriteCapability) -> bool {
        match capability {
            WriteCapability::Always => true,
            WriteCapability::Merge => self.supports_merge,
            WriteCapability::ColumnScopedMerge => self.supports_column_scoped_merge,
        }
    }
}

/// Whether the target backend can execute `pattern` at all (the fourth
/// admission factor alone).
pub fn pattern_capability_available(
    pattern: &WritePattern,
    backend: BackendWriteCapabilities,
) -> bool {
    backend.provides(pattern.capability)
}

/// Whether the output's declared contract facts satisfy `pattern`'s
/// structural requirements (the first admission factor alone).
pub fn pattern_facts_satisfied(pattern: &WritePattern, facts: OutputContractFacts) -> bool {
    pattern.required_facts.iter().all(|f| facts.satisfies(*f))
}

/// Whether `pattern` is admissible for an output declaring `facts`, on a
/// backend with `backend`'s write capabilities — factors one and four of
/// the available-addressings rule combined (`incremental_models.md`
/// §"The available-addressings rule"). This does not evaluate the
/// trigger/changed-input factor or the per-cell equivalence-invariant
/// factor (three and two): those need a specific cell's proven row
/// identity / column comparability, which this module's registry-level
/// admission check does not have — see [`resolve_write_pin`]'s
/// `cell_can_uphold_equivalence` parameter for where that factor is
/// threaded in by the caller.
pub fn pattern_admissible(
    pattern: &WritePattern,
    facts: OutputContractFacts,
    backend: BackendWriteCapabilities,
) -> bool {
    pattern_facts_satisfied(pattern, facts) && pattern_capability_available(pattern, backend)
}

/// The admissible write-pattern name set for an output declaring `facts`, on
/// a backend with `backend`'s capabilities — every registry entry whose
/// structural facts and capability are satisfied. Feeds `smelt explain`'s
/// admissible-set row and the cost model's candidate pool.
pub fn admissible_write_patterns(
    facts: OutputContractFacts,
    backend: BackendWriteCapabilities,
) -> Vec<&'static str> {
    WRITE_PATTERN_REGISTRY
        .iter()
        .filter(|p| pattern_admissible(p, facts, backend))
        .map(|p| p.name)
        .collect()
}

/// Why a `maintenance.cells[].write` pin was refused
/// (`incremental_models.md` §"User pins"). Never a silent downgrade: a
/// refused pin never resolves to a substituted pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WritePinRefusal {
    /// The name is not in the registry, or the target backend cannot
    /// provide the capability it needs.
    PatternUnavailable { pattern: String, backend: String },
    /// The name resolves in the registry and the backend can run it, but
    /// this cell's own facts (or a deeper equivalence proof the caller
    /// supplies) can't uphold its equivalence obligation — e.g. `keyed`
    /// pinned on an identity-free output.
    AddressingRefused {
        cell: String,
        pattern: String,
        why: String,
    },
}

impl std::fmt::Display for WritePinRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WritePinRefusal::PatternUnavailable { pattern, backend } => write!(
                f,
                "MaintenanceWritePatternUnavailable: write pattern '{pattern}' is unrecognised, \
                 or backend '{backend}' cannot provide it"
            ),
            WritePinRefusal::AddressingRefused { cell, pattern, why } => write!(
                f,
                "MaintenanceWriteAddressingRefused: write pattern '{pattern}' cannot uphold the \
                 equivalence invariant for cell {cell} — {why}"
            ),
        }
    }
}

/// Resolve a `write:` pin against the registry for one cell
/// (`incremental_models.md` §"User pins"). An unrecognised name, or one the
/// target backend cannot execute, is [`WritePinRefusal::PatternUnavailable`];
/// a name that is registry-recognised and backend-capable but whose
/// structural facts this output does not declare (or whose deeper
/// equivalence obligation `cell_can_uphold_equivalence` refuses) is
/// [`WritePinRefusal::AddressingRefused`]. The pin never widens the
/// admissible set: `Ok` only when the named pattern itself is the resolved
/// addressing.
pub fn resolve_write_pin(
    cell_label: &str,
    pin: &str,
    backend_name: &str,
    facts: OutputContractFacts,
    backend: BackendWriteCapabilities,
    cell_can_uphold_equivalence: impl Fn(&WritePattern) -> Result<(), String>,
) -> Result<&'static WritePattern, WritePinRefusal> {
    let Some(pattern) = lookup_write_pattern(pin) else {
        return Err(WritePinRefusal::PatternUnavailable {
            pattern: pin.to_string(),
            backend: backend_name.to_string(),
        });
    };
    if !pattern_capability_available(pattern, backend) {
        return Err(WritePinRefusal::PatternUnavailable {
            pattern: pin.to_string(),
            backend: backend_name.to_string(),
        });
    }
    if !pattern_facts_satisfied(pattern, facts) {
        return Err(WritePinRefusal::AddressingRefused {
            cell: cell_label.to_string(),
            pattern: pin.to_string(),
            why: format!(
                "the output does not declare the contract fact(s) '{pin}' requires \
                 ({:?})",
                pattern.required_facts
            ),
        });
    }
    if let Err(why) = cell_can_uphold_equivalence(pattern) {
        return Err(WritePinRefusal::AddressingRefused {
            cell: cell_label.to_string(),
            pattern: pin.to_string(),
            why,
        });
    }
    Ok(pattern)
}

/// The per-cell equivalence-invariant proof a `write:` pin's
/// `cell_can_uphold_equivalence` hook ([`resolve_write_pin`]) delegates to —
/// single owner of "does this cell's own derived facts uphold the pattern's
/// equivalence obligation" (`incremental_models.md` §"Per-cell write
/// addressing" → "User pins"). A **compare-based** pattern (`diff_patch`,
/// `keyed_conditional`, `staged_candidate` — the write mechanisms whose
/// physical form diffs a candidate row against prior state before writing)
/// delegates to [`choice::resolve_write_suppression`]'s P2 (row identity)/P3
/// (column comparability) proof and maps a
/// [`choice::WriteSuppression::Unconditional`] verdict to `Err`; every other
/// registry pattern (plain `region`/`keyed`/`column`/`update`/`full_rebuild`,
/// none of which compares against prior state) has no comparability
/// obligation at all and is unconditionally `Ok`.
pub fn cell_equivalence_proof(
    pattern: &WritePattern,
    group_columns: &[String],
    comparability: &[ColumnComparability],
    row_identity: &RowIdentityVerdict,
) -> Result<(), String> {
    match pattern.name {
        "diff_patch" | "keyed_conditional" | "staged_candidate" => {
            match choice::resolve_write_suppression(group_columns, comparability, row_identity) {
                choice::WriteSuppression::Suppressed { .. } => Ok(()),
                choice::WriteSuppression::Unconditional { why } => Err(why),
            }
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod cell_equivalence_proof_tests {
    use super::*;
    use crate::analysis::walk::Comparability;

    fn comparable(col: &str) -> ColumnComparability {
        ColumnComparability {
            output: col.to_string(),
            comparability: Comparability::Comparable,
        }
    }

    fn incomparable(col: &str) -> ColumnComparability {
        ColumnComparability {
            output: col.to_string(),
            comparability: Comparability::Incomparable,
        }
    }

    fn keyed_identity() -> RowIdentityVerdict {
        RowIdentityVerdict {
            identity: RowIdentity::Key(vec!["id".to_string()]),
            proven_mismatch: None,
        }
    }

    #[test]
    fn compare_based_pattern_refuses_an_incomparable_group() {
        let pattern = lookup_write_pattern("diff_patch").expect("diff_patch registered");
        let err = cell_equivalence_proof(
            pattern,
            &["amount".to_string()],
            &[incomparable("amount")],
            &keyed_identity(),
        )
        .expect_err("an incomparable compared column must refuse");
        assert!(
            err.contains("amount"),
            "refusal must name the incomparable column: {err}"
        );
    }

    #[test]
    fn compare_based_pattern_accepts_a_fully_comparable_group() {
        let pattern = lookup_write_pattern("keyed_conditional").expect("registered");
        cell_equivalence_proof(
            pattern,
            &["amount".to_string()],
            &[comparable("amount")],
            &keyed_identity(),
        )
        .expect("a fully comparable group over a proven key must be admitted");
    }

    #[test]
    fn region_and_full_rebuild_patterns_need_no_comparability_proof() {
        let region = lookup_write_pattern("region").expect("registered");
        let full_rebuild = lookup_write_pattern("full_rebuild").expect("registered");
        let whole_row = RowIdentityVerdict {
            identity: RowIdentity::WholeRow,
            proven_mismatch: None,
        };
        cell_equivalence_proof(region, &[], &[], &whole_row)
            .expect("region has no comparability obligation");
        cell_equivalence_proof(full_rebuild, &[], &[], &whole_row)
            .expect("full_rebuild has no comparability obligation");
    }

    #[test]
    fn compare_based_pattern_refuses_a_whole_row_cell() {
        let pattern = lookup_write_pattern("staged_candidate").expect("registered");
        let whole_row = RowIdentityVerdict {
            identity: RowIdentity::WholeRow,
            proven_mismatch: None,
        };
        let err = cell_equivalence_proof(
            pattern,
            &["amount".to_string()],
            &[comparable("amount")],
            &whole_row,
        )
        .expect_err("no proven row identity must refuse regardless of comparability");
        assert!(err.contains("row identity"), "got: {err}");
    }
}

#[cfg(test)]
mod write_pattern_registry_tests {
    use super::*;

    #[test]
    fn every_registered_pattern_resolves_by_name_with_declared_facts() {
        let names: Vec<&str> = WRITE_PATTERN_REGISTRY.iter().map(|p| p.name).collect();
        for expected in [
            "region",
            "keyed",
            "column",
            "update",
            "full_rebuild",
            "keyed_conditional",
            "staged_candidate",
            "diff_patch",
        ] {
            assert!(
                names.contains(&expected),
                "registry must contain '{expected}'; got {names:?}"
            );
            let pattern = lookup_write_pattern(expected)
                .unwrap_or_else(|| panic!("'{expected}' must resolve by name"));
            assert_eq!(pattern.name, expected);
        }
    }

    #[test]
    fn keyed_pattern_not_admissible_without_identity() {
        let no_identity = OutputContractFacts {
            has_identity: false,
            has_partition_axis: true,
        };
        let keyed = lookup_write_pattern("keyed").expect("keyed registered");
        assert!(
            !pattern_admissible(keyed, no_identity, BackendWriteCapabilities::all()),
            "keyed MERGE must not be admissible for an identity-free output"
        );

        let with_identity = OutputContractFacts {
            has_identity: true,
            has_partition_axis: false,
        };
        assert!(pattern_admissible(
            keyed,
            with_identity,
            BackendWriteCapabilities::all()
        ));
    }

    #[test]
    fn region_pattern_not_admissible_without_partition_axis() {
        let no_axis = OutputContractFacts {
            has_identity: true,
            has_partition_axis: false,
        };
        let region = lookup_write_pattern("region").expect("region registered");
        assert!(
            !pattern_admissible(region, no_axis, BackendWriteCapabilities::all()),
            "region DELETE+INSERT must not be admissible for a clockless output"
        );

        let with_axis = OutputContractFacts {
            has_identity: false,
            has_partition_axis: true,
        };
        assert!(pattern_admissible(
            region,
            with_axis,
            BackendWriteCapabilities::all()
        ));
    }

    #[test]
    fn column_and_keyed_conditional_gate_on_backend_capability() {
        let facts = OutputContractFacts {
            has_identity: true,
            has_partition_axis: false,
        };
        let no_merge = BackendWriteCapabilities {
            supports_merge: false,
            supports_column_scoped_merge: false,
        };
        let column = lookup_write_pattern("column").expect("column registered");
        let keyed_conditional =
            lookup_write_pattern("keyed_conditional").expect("keyed_conditional registered");
        assert!(!pattern_admissible(column, facts, no_merge));
        assert!(!pattern_admissible(keyed_conditional, facts, no_merge));

        // staged_candidate is the merge-less realisation — admissible with
        // identity alone, no backend capability required.
        let staged_candidate =
            lookup_write_pattern("staged_candidate").expect("staged_candidate registered");
        assert!(pattern_admissible(staged_candidate, facts, no_merge));
    }

    #[test]
    fn full_rebuild_always_admissible() {
        let facts = OutputContractFacts::default();
        let no_caps = BackendWriteCapabilities::default();
        let full_rebuild = lookup_write_pattern("full_rebuild").expect("full_rebuild registered");
        assert!(pattern_admissible(full_rebuild, facts, no_caps));
    }

    #[test]
    fn unrecognised_pin_refuses_pattern_unavailable() {
        let err = resolve_write_pin(
            "cell-1",
            "not_a_real_pattern",
            "duckdb",
            OutputContractFacts::default(),
            BackendWriteCapabilities::all(),
            |_| Ok(()),
        )
        .expect_err("unrecognised name must refuse");
        assert!(matches!(err, WritePinRefusal::PatternUnavailable { .. }));
        assert!(err
            .to_string()
            .contains("MaintenanceWritePatternUnavailable"));
    }

    #[test]
    fn backend_incapable_pin_refuses_pattern_unavailable() {
        let facts = OutputContractFacts {
            has_identity: true,
            has_partition_axis: false,
        };
        let no_merge = BackendWriteCapabilities::default();
        let err = resolve_write_pin("cell-1", "keyed", "spark_parquet", facts, no_merge, |_| {
            Ok(())
        })
        .expect_err("a backend without MERGE must refuse keyed as unavailable");
        assert!(matches!(err, WritePinRefusal::PatternUnavailable { .. }));
    }

    #[test]
    fn identity_free_keyed_pin_refuses_addressing_refused_never_downgrades() {
        let no_identity = OutputContractFacts {
            has_identity: false,
            has_partition_axis: true,
        };
        let err = resolve_write_pin(
            "cell-1",
            "keyed",
            "duckdb",
            no_identity,
            BackendWriteCapabilities::all(),
            |_| Ok(()),
        )
        .expect_err("keyed on an identity-free output must refuse addressing");
        match &err {
            WritePinRefusal::AddressingRefused { cell, pattern, .. } => {
                assert_eq!(cell, "cell-1");
                assert_eq!(pattern, "keyed");
            }
            other => panic!("expected AddressingRefused, got {other:?}"),
        }
        assert!(err
            .to_string()
            .contains("MaintenanceWriteAddressingRefused"));
    }

    #[test]
    fn deeper_equivalence_refusal_is_addressing_refused() {
        let facts = OutputContractFacts {
            has_identity: true,
            has_partition_axis: false,
        };
        let err = resolve_write_pin(
            "cell-1",
            "keyed",
            "duckdb",
            facts,
            BackendWriteCapabilities::all(),
            |_| Err("no proven row identity for this cell".to_string()),
        )
        .expect_err("caller-supplied equivalence refusal must propagate");
        assert!(matches!(err, WritePinRefusal::AddressingRefused { .. }));
    }

    #[test]
    fn valid_pin_resolves_and_never_widens_the_admissible_set() {
        let facts = OutputContractFacts {
            has_identity: false,
            has_partition_axis: true,
        };
        let resolved = resolve_write_pin(
            "cell-1",
            "region",
            "duckdb",
            facts,
            BackendWriteCapabilities::all(),
            |_| Ok(()),
        )
        .expect("region pin resolves for a partition-axis output");
        assert_eq!(resolved.name, "region");
    }
}

#[cfg(test)]
mod comparability_contract_tests {
    use super::*;
    use crate::analysis::walk::Comparability;
    use std::collections::BTreeMap;

    #[test]
    fn plausible_contract_forces_incomparable_regardless_of_walk_verdict() {
        let walk_comparability = vec![
            ColumnComparability {
                output: "amount".to_string(),
                comparability: Comparability::Comparable,
            },
            ColumnComparability {
                output: "notes".to_string(),
                comparability: Comparability::Comparable,
            },
        ];
        let mut contracts = BTreeMap::new();
        contracts.insert(
            "notes".to_string(),
            smelt_core::metadata::Contract::Plausible,
        );

        let out = column_comparability_with_contract(&walk_comparability, &contracts);

        assert_eq!(
            out.iter()
                .find(|c| c.output == "notes")
                .map(|c| c.comparability),
            Some(Comparability::Incomparable),
            "a plausible-contract column must be forced Incomparable regardless of \
             the walk's own (Comparable) verdict; got {out:?}"
        );
        assert_eq!(
            out.iter()
                .find(|c| c.output == "amount")
                .map(|c| c.comparability),
            Some(Comparability::Comparable),
            "a column with no plausible declaration must pass through unchanged; got {out:?}"
        );
    }

    #[test]
    fn no_contract_passes_the_walk_verdict_through_unchanged() {
        let walk_comparability = vec![ColumnComparability {
            output: "ts".to_string(),
            comparability: Comparability::Incomparable,
        }];
        let out = column_comparability_with_contract(&walk_comparability, &BTreeMap::new());
        assert_eq!(
            out, walk_comparability,
            "no declared contracts must leave the walk's verdict untouched"
        );
    }
}
