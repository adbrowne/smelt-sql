use super::*;

/// C2 (`docs/specs/property_diff.md` §Constraints item 6, Δ1): patch an
/// `added`/`removed` entry's `cause.reason` from the matching side's
/// per-model derivation-failure map, when that side's absence was a
/// FAILURE rather than a genuine new/deleted model. Single-owned here
/// (`docs/outcomes/20260905-property-diff/phases/05-plan.md` fix round 1,
/// Q6) rather than in the CLI, since this is a §Semantics rule Phase 6/7
/// must apply identically, not a CLI-only presentation choice.
pub fn apply_failure_reasons(
    diff: &mut PropertyDiff,
    base_failures: &BTreeMap<String, String>,
    work_failures: &BTreeMap<String, String>,
) {
    for model_diff in diff.models.iter_mut() {
        match model_diff.cause.kind {
            // "added" = present in the working tree, ABSENT from the
            // baseline — an absence-was-really-a-failure reason, if any,
            // is recorded on the BASELINE side.
            CauseKind::Added => {
                if let Some(reason) = base_failures.get(&model_diff.model) {
                    model_diff.cause.reason = Some(reason.clone());
                }
            }
            // "removed" = present in the baseline, ABSENT from the working
            // tree — the failure, if any, is on the WORKING TREE side.
            CauseKind::Removed => {
                if let Some(reason) = work_failures.get(&model_diff.model) {
                    model_diff.cause.reason = Some(reason.clone());
                }
            }
            CauseKind::Edited | CauseKind::Downstream => {}
        }
    }
}

/// The working-tree graph plus the edit provenance the diff attributes with
/// (`docs/specs/property_diff.md` §"Attribution"). Built by the caller
/// (a later phase) — `diff_profiles` never touches git.
///
/// `upstream` carries **model and source** edges: `DependencyGraph::
/// get_upstream` returns model deps only (`build` deliberately drops
/// `smelt.sources.*` refs, `smelt-core/src/graph.rs`), but attribution must
/// walk to "every edited model **or source**"
/// (`docs/specs/property_diff.md` §"Attribution").
#[derive(Debug, Clone, Default)]
pub struct DiffGraph {
    /// name -> direct upstream names (models and sources) it references.
    pub upstream: BTreeMap<String, Vec<String>>,
    pub edited: BTreeSet<String>,
    pub project_config_changed: bool,
}

impl DiffGraph {
    /// Build a [`DiffGraph`] from a loaded [`DependencyGraph`], adding back
    /// the source edges `DependencyGraph::build` drops. A source name is
    /// its bare dot-path with the leading `sources` segment stripped (the
    /// same convention `smelt-cli::explain::find_source_info` and
    /// `PropertySet::source_bounds` use), so an edited source and a
    /// `source_bound` change key against the same name.
    pub fn from_dependency_graph(
        g: &DependencyGraph,
        edited: BTreeSet<String>,
        project_config_changed: bool,
    ) -> Self {
        let mut upstream: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (name, model) in g.iter_models() {
            let mut deps = g.get_upstream(name);
            for r in &model.refs {
                let SmeltRef::Path(segs) = &r.smelt_ref;
                if let Some((first, rest)) = segs.split_first() {
                    if first == "sources" && !rest.is_empty() {
                        let bare = rest.join(".");
                        if !deps.contains(&bare) {
                            deps.push(bare);
                        }
                    }
                }
            }
            deps.sort();
            deps.dedup();
            upstream.insert(name.to_string(), deps);
        }
        DiffGraph {
            upstream,
            edited,
            project_config_changed,
        }
    }

    /// Attribute `model`'s shift (`docs/specs/property_diff.md`
    /// §"Attribution"): BFS upward over `upstream` from `model`, stopping
    /// at the first edited node on each path (never passing through it).
    /// Own file edited ⇒ `Edited`. No edited ancestor reached and
    /// `project_config_changed` ⇒ `Downstream` with `of: []` and the
    /// model-level reason. No edited ancestor and no config change is not
    /// expected to be called (a model cannot shift with no cause), but
    /// resolves to the same `of: []` shape rather than panicking
    /// (fail-loud discipline never demands a panic where a value suffices).
    pub fn attribute(&self, model: &str) -> Cause {
        if self.edited.contains(model) {
            return Cause {
                kind: CauseKind::Edited,
                of: vec![],
                reason: None,
            };
        }
        let mut visited: BTreeSet<String> = BTreeSet::new();
        let mut queue: VecDeque<String> = VecDeque::new();
        visited.insert(model.to_string());
        queue.push_back(model.to_string());
        let mut ancestors: BTreeSet<String> = BTreeSet::new();
        while let Some(current) = queue.pop_front() {
            let Some(ups) = self.upstream.get(&current) else {
                continue;
            };
            for up in ups {
                if !visited.insert(up.clone()) {
                    continue;
                }
                if self.edited.contains(up) {
                    ancestors.insert(up.clone());
                } else {
                    queue.push_back(up.clone());
                }
            }
        }
        if ancestors.is_empty() {
            Cause {
                kind: CauseKind::Downstream,
                of: vec![],
                reason: if self.project_config_changed {
                    Some("project configuration changed".to_string())
                } else {
                    None
                },
            }
        } else {
            Cause {
                kind: CauseKind::Downstream,
                of: ancestors.into_iter().collect(),
                reason: None,
            }
        }
    }
}
