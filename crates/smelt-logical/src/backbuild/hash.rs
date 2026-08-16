//! `plan_hash`: a pure, deterministic hash over a derived [`super::MigrationPlan`]
//! plus the [`super::BackbuildInputs`] that justified it — approval binds to
//! this hash, not to rendered SQL alone
//! (`docs/specs/definition_deltas.md` §Design "The plan hash covers the plan
//! data structure, not only rendered SQL"). Length-prefixed canonical
//! encoder, mirroring `smelt-fingerprint::hash::Encoder` — that crate's
//! `Encoder` is `pub(crate)` to its own module, so this is a local copy, not
//! a widened export. Pure and total: no I/O, no clock, exhaustive over every
//! enum this module encodes so a new variant fails to compile here rather
//! than silently hashing as its neighbour (fail-loud discipline,
//! `docs/specs/architecture.md` §"Fail-loud discipline").

use sha2::{Digest, Sha256};

use super::{
    BackbuildInputs, BackbuildOption, BackbuildRefusal, ColumnGroupPlan, CostClass, HSlot,
    MigrationPlan, SourceRef, Technique, TechniqueCandidate, Verdict, WriteScope,
};

/// Builds the canonical byte encoding incrementally. Every piece is written
/// with an unambiguous, length-prefixed framing so two structurally
/// different inputs can never collide on encoded bytes.
#[derive(Default)]
struct Encoder {
    buf: Vec<u8>,
}

impl Encoder {
    /// Write a tagged, length-prefixed string field.
    fn field(&mut self, tag: &str, value: &str) {
        self.raw(tag);
        self.raw_len(value.as_bytes());
    }

    /// Write a tag with no value (an enum discriminant, or a boolean flag's
    /// presence).
    fn tag(&mut self, tag: &str) {
        self.raw(tag);
    }

    fn raw(&mut self, s: &str) {
        self.buf.extend_from_slice(b"|");
        self.buf.extend_from_slice(s.as_bytes());
        self.buf.extend_from_slice(b":");
    }

    fn raw_len(&mut self, bytes: &[u8]) {
        self.buf
            .extend_from_slice(format!("{}=", bytes.len()).as_bytes());
        self.buf.extend_from_slice(bytes);
    }

    /// `sha256:<hex12>` — a short, display-friendly digest prefix. Collision
    /// risk at 48 bits is irrelevant here: the hash gates a human approval
    /// step over a small, reviewed plan, not an adversarial namespace.
    fn finish(self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(&self.buf);
        let digest = hasher.finalize();
        let hex: String = digest[..6].iter().map(|b| format!("{b:02x}")).collect();
        format!("sha256:{hex}")
    }
}

/// Hash `inputs` and the `plan` derived from them — the value
/// `smelt migrate` records as the approved plan hash and `--apply` re-derives
/// to check for drift. Two calls with the same (inputs, plan) content always
/// produce the same hash, regardless of map/set insertion order (every
/// container `BackbuildInputs`/`MigrationPlan` carry is already a `BTreeMap`/
/// `BTreeSet` or a plan-ordered `Vec`).
pub fn plan_hash(inputs: &BackbuildInputs, plan: &MigrationPlan) -> String {
    let mut enc = Encoder::default();
    encode_inputs(&mut enc, inputs);
    encode_plan(&mut enc, plan);
    enc.finish()
}

fn encode_bool(enc: &mut Encoder, tag: &str, value: bool) {
    enc.field(tag, if value { "true" } else { "false" });
}

fn encode_inputs(enc: &mut Encoder, inputs: &BackbuildInputs) {
    enc.tag("inputs");
    enc.field("table", &inputs.table);
    enc.field("after_sql", &inputs.after_sql);
    match &inputs.row_identity {
        Some(cols) => {
            enc.tag("row_identity");
            for c in cols {
                enc.field("row_identity_col", c);
            }
        }
        None => enc.tag("no_row_identity"),
    }
    for c in &inputs.not_null_columns {
        enc.field("not_null", c);
    }
    for (name, ty) in &inputs.added_column_types {
        enc.field("added_col", name);
        enc.field("added_col_type", ty);
    }
    for (name, source) in &inputs.sources {
        enc.field("source", name);
        encode_source_ref(enc, source);
    }
}

fn encode_source_ref(enc: &mut Encoder, source: &SourceRef) {
    enc.field("source_physical_name", &source.physical_name);
    match &source.unique_key {
        Some(cols) => {
            enc.tag("source_unique_key");
            for c in cols {
                enc.field("source_unique_key_col", c);
            }
        }
        None => enc.tag("source_no_unique_key"),
    }
    for c in &source.not_null_columns {
        enc.field("source_not_null", c);
    }
}

fn encode_plan(enc: &mut Encoder, plan: &MigrationPlan) {
    enc.tag("plan");
    encode_bool(enc, "eclipsed", plan.eclipsed);
    for group in &plan.groups {
        encode_group(enc, group);
    }
    encode_option(enc, &plan.full_refresh);
}

fn encode_group(enc: &mut Encoder, group: &ColumnGroupPlan) {
    enc.tag("group");
    enc.field("group_label", &group.label);
    enc.tag(verdict_tag(group.verdict));
    for candidate in &group.candidates {
        encode_candidate(enc, candidate);
    }
    for refusal in &group.refusals {
        encode_refusal(enc, refusal);
    }
}

fn encode_candidate(enc: &mut Encoder, candidate: &TechniqueCandidate) {
    enc.tag("candidate");
    enc.tag(technique_tag(candidate.technique));
    enc.tag(cost_class_tag(candidate.cost_class));
    for stmt in &candidate.statements {
        enc.field("statement", stmt);
    }
    encode_bool(enc, "reads_upstream", candidate.reads_upstream);
    encode_bool(enc, "rerun_safe", candidate.rerun_safe);
}

fn encode_refusal(enc: &mut Encoder, refusal: &BackbuildRefusal) {
    enc.tag("refusal");
    enc.field("refusal_atom", &refusal.atom);
    enc.field("refusal_reason", &refusal.reason);
}

fn encode_option(enc: &mut Encoder, option: &BackbuildOption) {
    enc.tag("option");
    enc.tag(technique_tag(option.technique));
    match option.slot {
        Some(slot) => enc.tag(hslot_tag(slot)),
        None => enc.tag("no_slot"),
    }
    for stmt in &option.statements {
        enc.field("statement", stmt);
    }
    enc.tag(write_scope_tag(option.write_scope));
    encode_bool(enc, "reads_upstream", option.reads_upstream);
    encode_bool(enc, "rerun_safe", option.rerun_safe);
}

fn verdict_tag(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Eclipsed => "verdict_eclipsed",
        Verdict::BackfillInPlace => "verdict_backfill_in_place",
        Verdict::ReDerive => "verdict_re_derive",
        Verdict::SkeletonChange => "verdict_skeleton_change",
    }
}

fn cost_class_tag(cost_class: CostClass) -> &'static str {
    match cost_class {
        CostClass::Metadata => "cost_metadata",
        CostClass::LocalColumnUpdate => "cost_local_column_update",
        CostClass::UpstreamColumnUpdate => "cost_upstream_column_update",
        CostClass::LocalRowSubset => "cost_local_row_subset",
        CostClass::UpstreamRowSubset => "cost_upstream_row_subset",
        CostClass::Destructive => "cost_destructive",
        CostClass::FullTable => "cost_full_table",
    }
}

fn technique_tag(technique: Technique) -> &'static str {
    match technique {
        Technique::FullRefresh => "technique_full_refresh",
        Technique::SelfDerivedColumnAdd => "technique_self_derived_column_add",
        Technique::Rename => "technique_rename",
        Technique::SelfDerivedColumnRewrite => "technique_self_derived_column_rewrite",
        Technique::UpstreamPullthrough => "technique_upstream_pullthrough",
        Technique::JoinEnrichmentUpdateFrom => "technique_join_enrichment_update_from",
        Technique::JoinEnrichmentScalarSubquery => "technique_join_enrichment_scalar_subquery",
        Technique::PredicateTightenDelete => "technique_predicate_tighten_delete",
        Technique::HorizonExtensionInsert => "technique_horizon_extension_insert",
        Technique::FilterLoosenInsert => "technique_filter_loosen_insert",
        Technique::UnionBranchInsert => "technique_union_branch_insert",
        Technique::DiscriminatedBranchDelete => "technique_discriminated_branch_delete",
        Technique::AggregateColumnBackfill => "technique_aggregate_column_backfill",
        Technique::WindowColumnBackfill => "technique_window_column_backfill",
        Technique::ColumnDrop => "technique_column_drop",
    }
}

fn write_scope_tag(write_scope: WriteScope) -> &'static str {
    match write_scope {
        WriteScope::None => "write_scope_none",
        WriteScope::ColumnScoped => "write_scope_column_scoped",
        WriteScope::RowSubset => "write_scope_row_subset",
        WriteScope::FullWrite => "write_scope_full_write",
        WriteScope::Destructive => "write_scope_destructive",
    }
}

fn hslot_tag(hslot: HSlot) -> &'static str {
    match hslot {
        HSlot::Rename => "hslot_rename",
        HSlot::Alter => "hslot_alter",
        HSlot::Delete => "hslot_delete",
        HSlot::UpdateMerge => "hslot_update_merge",
        HSlot::Insert => "hslot_insert",
        HSlot::Drop => "hslot_drop",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    fn full_refresh_option() -> BackbuildOption {
        BackbuildOption {
            technique: Technique::FullRefresh,
            slot: None,
            statements: vec!["CREATE OR REPLACE TABLE t AS SELECT 1".to_string()],
            write_scope: WriteScope::FullWrite,
            reads_upstream: true,
            rerun_safe: true,
        }
    }

    fn candidate(technique: Technique, statement: &str) -> TechniqueCandidate {
        TechniqueCandidate {
            technique,
            cost_class: CostClass::LocalColumnUpdate,
            statements: vec![statement.to_string()],
            reads_upstream: false,
            rerun_safe: true,
        }
    }

    fn group(verdict: Verdict, candidates: Vec<TechniqueCandidate>) -> ColumnGroupPlan {
        ColumnGroupPlan {
            label: "added column 'net_amount'".to_string(),
            verdict,
            candidates,
            refusals: Vec::new(),
        }
    }

    fn source_ref(unique_key: Option<Vec<&str>>) -> SourceRef {
        SourceRef {
            physical_name: "orders".to_string(),
            unique_key: unique_key.map(|k| k.into_iter().map(str::to_string).collect()),
            not_null_columns: BTreeSet::new(),
        }
    }

    fn inputs_with_sources(sources: BTreeMap<String, SourceRef>) -> BackbuildInputs {
        BackbuildInputs {
            table: "orders_summary".to_string(),
            after_sql: "SELECT id, amount * 0.9 AS net_amount FROM orders".to_string(),
            row_identity: None,
            not_null_columns: BTreeSet::new(),
            added_column_types: BTreeMap::new(),
            sources,
        }
    }

    fn plan_with(
        verdict: Verdict,
        candidate_statement: &str,
        technique: Technique,
    ) -> MigrationPlan {
        MigrationPlan {
            eclipsed: false,
            groups: vec![group(
                verdict,
                vec![candidate(technique, candidate_statement)],
            )],
            full_refresh: full_refresh_option(),
        }
    }

    #[test]
    fn plan_hash_is_stable_across_repeated_derivation() {
        let inputs = inputs_with_sources(BTreeMap::new());
        let plan = plan_with(
            Verdict::BackfillInPlace,
            "UPDATE t SET net_amount = amount * 0.9",
            Technique::SelfDerivedColumnAdd,
        );

        assert_eq!(plan_hash(&inputs, &plan), plan_hash(&inputs, &plan));
    }

    #[test]
    fn plan_hash_changes_when_statement_text_changes() {
        let inputs = inputs_with_sources(BTreeMap::new());
        let a = plan_with(
            Verdict::BackfillInPlace,
            "UPDATE t SET net_amount = amount * 0.9",
            Technique::SelfDerivedColumnAdd,
        );
        let b = plan_with(
            Verdict::BackfillInPlace,
            "UPDATE t SET net_amount = amount * 0.8",
            Technique::SelfDerivedColumnAdd,
        );

        assert_ne!(plan_hash(&inputs, &a), plan_hash(&inputs, &b));
    }

    #[test]
    fn plan_hash_changes_when_source_facts_change() {
        let plan = plan_with(
            Verdict::ReDerive,
            "UPDATE t SET region = u.region FROM orders u WHERE t.id = u.id",
            Technique::UpstreamPullthrough,
        );

        let mut without_key = BTreeMap::new();
        without_key.insert("orders".to_string(), source_ref(None));
        let mut with_key = BTreeMap::new();
        with_key.insert("orders".to_string(), source_ref(Some(vec!["id"])));

        let a = inputs_with_sources(without_key);
        let b = inputs_with_sources(with_key);

        assert_ne!(plan_hash(&a, &plan), plan_hash(&b, &plan));
    }

    #[test]
    fn plan_hash_changes_when_verdict_changes() {
        let inputs = inputs_with_sources(BTreeMap::new());
        let backfill = plan_with(
            Verdict::BackfillInPlace,
            "UPDATE t SET net_amount = amount * 0.9",
            Technique::SelfDerivedColumnAdd,
        );
        let mut skeleton = backfill.clone();
        skeleton.groups[0].verdict = Verdict::SkeletonChange;
        skeleton.groups[0].candidates = Vec::new();

        assert_ne!(plan_hash(&inputs, &backfill), plan_hash(&inputs, &skeleton));
    }

    #[test]
    fn plan_hash_is_order_independent_over_sources() {
        let plan = plan_with(
            Verdict::ReDerive,
            "UPDATE t SET region = u.region FROM orders u WHERE t.id = u.id",
            Technique::UpstreamPullthrough,
        );

        let mut first = BTreeMap::new();
        first.insert("orders".to_string(), source_ref(Some(vec!["id"])));
        first.insert("customers".to_string(), source_ref(Some(vec!["cust_id"])));

        let mut second = BTreeMap::new();
        second.insert("customers".to_string(), source_ref(Some(vec!["cust_id"])));
        second.insert("orders".to_string(), source_ref(Some(vec!["id"])));

        let a = inputs_with_sources(first);
        let b = inputs_with_sources(second);

        assert_eq!(plan_hash(&a, &plan), plan_hash(&b, &plan));
    }
}
