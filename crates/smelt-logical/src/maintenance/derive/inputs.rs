use super::*;

/// Everything the v0 derivation reads. `column_groups` and
/// `output.skeleton_columns` are hand-supplied (the deferred classifiers);
/// the rest is derived from `sql` and the source declarations.
#[derive(Debug)]
pub struct ModelInputs<'a> {
    /// Expanded model SQL (functions inlined), used for bound derivation.
    pub sql: &'a str,
    pub output: OutputSpec,
    pub sources: Vec<SourceFacts>,
    pub column_groups: Vec<ColumnGroup>,
    /// Present for keyed-grain models whose new-data cell should fold.
    pub fold: Option<FoldSpec>,
    /// The model's existing (pre-`ColumnAdded`) output columns — the
    /// [`crate::analysis::definition_change::classify_definition_change`]
    /// proof's `old_columns` for a `ColumnAdded` trigger. Empty means "no
    /// old schema known", which fails closed exactly as the retired
    /// `column_add_proof: None` did (`docs/plans/
    /// 20260808-derived-maintenance-proofs.md` Phase 4).
    pub old_columns: Vec<ColumnDef>,
    /// The model's previously-deployed source SQL text (world-fact, from
    /// the deployed-schema snapshot's `model_sql`, supplied by the caller —
    /// this module does no I/O of its own). `None` means "no known deployed
    /// SQL", which derives no [`Refusal::SkeletonClauseChanged`] at all —
    /// fail-closed, the same posture `old_columns`/`deployed_column_names`
    /// take (`docs/specs/definition_deltas.md` §"Detection").
    pub old_sql: Option<&'a str>,
    /// The declared `timeseries.partition_column` of a `Grain::Key` model,
    /// `None` for a `Grain::Partition` output or a keyed output with no
    /// declared `timeseries:` block. Poses the footprint question
    /// (`model_properties.md` §"Footprint reflection / bounded write
    /// footprint") for a keyed-grain output against this axis; a bare
    /// keyed output (`None` here) gets no footprint claim at all.
    pub keyed_time_axis: Option<&'a str>,
    /// The declared `timeseries.partition_column` recorded in the
    /// deployed-schema snapshot at last deploy (world-fact, supplied by the
    /// caller — this module does no I/O of its own). `None` means "no known
    /// recorded address" (no snapshot, or one written before this field
    /// existed), which derives no [`Refusal::PartitionColumnChanged`] at
    /// all — fail-closed, the same posture `old_sql` takes.
    pub old_partition_col: Option<&'a str>,
}

impl ModelInputs<'_> {
    pub(super) fn source(&self, name: &str) -> Option<&SourceFacts> {
        self.sources.iter().find(|s| s.name == name)
    }

    pub(super) fn bound_context(&self) -> BoundContext {
        let mut ctx = BoundContext::new();
        for s in &self.sources {
            if let Some(p) = &s.partition_col {
                ctx.add_source(&s.name, p);
            }
        }
        ctx
    }

    pub(super) fn output_partition_col(&self) -> Option<&str> {
        match &self.output.grain {
            Grain::Partition { partition_col } => Some(partition_col),
            Grain::Key { .. } | Grain::Succession { .. } => None,
        }
    }

    /// The declared identity off the output's own grain (P2, `row_identity`):
    /// `Grain::Key`'s `unique_key`, or nothing for `Grain::Partition` — a
    /// partition-grain output declares no row-level identity through
    /// `Grain` itself.
    pub(super) fn declared_unique_key(&self) -> &[String] {
        match &self.output.grain {
            Grain::Key { unique_key } => unique_key,
            Grain::Partition { .. } | Grain::Succession { .. } => &[],
        }
    }
}

/// How one read source relates to the output's partition axis for a
/// region-anchored maintenance op.
pub enum SourceLink {
    /// Bounded: the derived scan clamp, anchored to the output region.
    Clamp(ScanClamp),
    /// Clocked but with no derivable link to the output partition axis (or
    /// an unbounded one) — the op cannot be partition-pruned.
    Unlinked { why: String },
    /// Not clocked at all: a lookup read in full.
    Unclocked,
}

/// The three per-source derivations the partition-locality projection
/// composes (`model_properties.md` §"Partition-locality projection") —
/// read bounds ([`derive_model_bounds`]), reflected write footprints
/// ([`reflect_footprint`]), and cross-axis predicate evidence
/// ([`derive_cross_axis_links`]) — derived once per model (or per edge
/// context) and threaded together to every clamp-construction site.
pub struct LocalityInputs<'a> {
    pub bounds: &'a HashMap<String, BoundResult>,
    pub footprints: &'a HashMap<String, FootprintResult>,
    pub links: &'a HashMap<String, CrossAxisLink>,
}

/// Project one source's partition-locality verdict onto a [`SourceLink`] —
/// the adapter between the pure locality proof
/// ([`locality_verdict`] — `model_properties.md` §"Partition-locality
/// projection") and the clamp/refusal plumbing the derivation sites share.
///
/// For a **partition-addressed** output, the verdict is the proof's,
/// verbatim: read bound ([`derive_model_bounds`]) AND reflected write
/// footprint ([`reflect_footprint`]) must both be `Bounded`, and a
/// cross-axis source (its partition column is not the output's) is linked
/// only by the explicit-predicate evidence
/// ([`derive_cross_axis_links`]) — never by a nonzero scan margin. A
/// `Local` verdict over a `Bounded` read constructs the derived
/// [`ScanClamp`] exactly as before; every `NotLocal` reason becomes the
/// `Unlinked` why. Policy (creation cells never refuse; the K8
/// `ScanUnbounded` refusal honoring `allow_full_scan`) stays at each call
/// site — this adapter is policy-free.
///
/// For a **keyed-grain** output (`output_partition_col` is `None`) with a
/// declared `keyed_time_axis`, the footprint question IS posed against that
/// axis — `loc.footprints` must already be `reflect_footprint`'s verdicts
/// against it (the caller's responsibility, exactly as for a
/// partition-addressed output): a `Bounded` footprint constructs the clamp
/// carrying the derived write footprint; `Unbounded`/`NotDerivable` refuses
/// the link. With no declared `keyed_time_axis` at all, the pre-proof
/// linking rule is kept verbatim (a nonzero-margin `Bounded` read links, a
/// zero-margin one does not), but the resulting clamp carries no footprint
/// claim (`write_footprint: None`) — there is no axis to have posed it
/// against.
pub fn project_source_link(
    output_partition_col: Option<&str>,
    keyed_time_axis: Option<&str>,
    loc: &LocalityInputs<'_>,
    facts: &SourceFacts,
) -> SourceLink {
    let LocalityInputs {
        bounds,
        footprints,
        links,
    } = loc;
    let Some(col) = &facts.partition_col else {
        return SourceLink::Unclocked;
    };
    let Some(output_axis) = output_partition_col else {
        let read = bounds.get(&facts.name);
        return match keyed_time_axis {
            Some(_axis) => {
                let Some(BoundResult::Bounded { before, after, .. }) = read else {
                    return SourceLink::Unlinked {
                        why: "scan bound not derivable".to_string(),
                    };
                };
                match footprints
                    .get(&facts.name)
                    .unwrap_or(&FootprintResult::NotDerivable)
                {
                    FootprintResult::Bounded {
                        before: write_before,
                        after: write_after,
                        ..
                    } => SourceLink::Clamp(ScanClamp {
                        source: facts.name.clone(),
                        column: col.clone(),
                        before: *before,
                        after: *after,
                        write_footprint: Some((*write_before, *write_after)),
                    }),
                    FootprintResult::Unbounded => SourceLink::Unlinked {
                        why: "derived write footprint is unbounded against the declared time \
                              axis — a stored trajectory column"
                            .to_string(),
                    },
                    FootprintResult::NotDerivable => SourceLink::Unlinked {
                        why: "write footprint not derivable against the declared time axis"
                            .to_string(),
                    },
                }
            }
            None => match read {
                Some(BoundResult::Bounded { before, after, .. }) => {
                    if *before > Seconds::ZERO || *after > Seconds::ZERO {
                        SourceLink::Clamp(ScanClamp {
                            source: facts.name.clone(),
                            column: col.clone(),
                            before: *before,
                            after: *after,
                            write_footprint: None,
                        })
                    } else {
                        SourceLink::Unlinked {
                            why: format!(
                                "no predicate links '{col}' to the output partition axis — the \
                                 scan cannot be partition-pruned"
                            ),
                        }
                    }
                }
                Some(BoundResult::Unbounded) => SourceLink::Unlinked {
                    why: "derived scan is unbounded".to_string(),
                },
                Some(BoundResult::NotDerivable) | None => SourceLink::Unlinked {
                    why: "scan bound not derivable".to_string(),
                },
            },
        };
    };

    let read = bounds
        .get(&facts.name)
        .unwrap_or(&BoundResult::NotDerivable);
    // `bounds` and `footprints` are derived from the same SQL over the same
    // context, so their key sets agree; a source absent from both is refused
    // through the read leg (`NotDerivable`) before the footprint is read.
    let footprint = footprints
        .get(&facts.name)
        .unwrap_or(&FootprintResult::NotDerivable);
    let link = links
        .get(&facts.name)
        .copied()
        .unwrap_or(CrossAxisLink::Absent);

    match locality_verdict(read, footprint, Some(col), Some(output_axis), link) {
        LocalityVerdict::Local => match (read, footprint) {
            (
                BoundResult::Bounded { before, after, .. },
                FootprintResult::Bounded {
                    before: write_before,
                    after: write_after,
                    ..
                },
            ) => SourceLink::Clamp(ScanClamp {
                source: facts.name.clone(),
                column: col.clone(),
                before: *before,
                after: *after,
                write_footprint: Some((*write_before, *write_after)),
            }),
            // Unreachable: the proof only ever answers `Local` over a
            // `Bounded` read AND a `Bounded` footprint; kept fail-closed
            // rather than panicking.
            _ => SourceLink::Unlinked {
                why: "scan bound not derivable".to_string(),
            },
        },
        LocalityVerdict::NotLocal { reason } => SourceLink::Unlinked { why: reason },
    }
}
