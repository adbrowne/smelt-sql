//! Live observed-delta consumption (`docs/outcomes/
//! 20260816-scheduler-delta-signatures/outcome.md` phase 6): the backend
//! read `--since-upstream` was always missing — `propagation::
//! observed_delta_keys_to_read` (pure) says WHICH `(model, window)` keys
//! matter; this module reads them off the warehouse and assembles the
//! `ObservedDeltaLookup` `propagation::plan_since_upstream_with_observed_deltas`
//! consumes.

use std::collections::BTreeMap;

use anyhow::Result;
use smelt_backend::Backend;
use smelt_core::sources::SourceInfo;
use smelt_core::ModelFile;
use smelt_logical::maintenance::propagate::KeyValues;

use crate::maintenance_driver::{diff_repair_group_sidecar_changed_keys, read_observed_delta};
use crate::propagation::{
    fold_keyed_seed_values, keyed_seed_diff_result_to_key_values, keyed_seed_diffs_to_read,
    observed_delta_keys_to_read, plan_since_upstream_live, KeyedSeedDiff, ObservedDeltaKey,
    ObservedDeltaLookup, SinceUpstreamPlan, SourceDelta,
};

/// Read every key in `keys` off `backend`, one `read_observed_delta` call
/// each. An absent key (`read_observed_delta` returns `None` — never
/// recorded) is simply omitted from the returned map — absent is not the
/// same as present-and-empty (`incremental_models.md` §"The graph layer" —
/// "Empty and absent are distinct"), and `ObservedDeltaLookup`'s own `Option`
/// semantics at the consuming call site already encode that distinction via
/// map membership.
pub async fn resolve_observed_delta_lookup(
    backend: &dyn Backend,
    schema: &str,
    keys: &[ObservedDeltaKey],
) -> Result<ObservedDeltaLookup> {
    let mut lookup = ObservedDeltaLookup::new();
    for key in keys {
        let (model, window_start, window_end) = key;
        if let Some(delta) = read_observed_delta(backend, schema, model, window_start, window_end)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?
        {
            lookup.insert(key.clone(), delta);
        }
    }
    Ok(lookup)
}

/// Live keyed-seed resolution (`docs/outcomes/
/// 20260816-scheduler-delta-signatures/phases/07-plan.md`): read the
/// group-grain sidecar diff [`crate::propagation::keyed_seed_diffs_to_read`]
/// says matters, one `diff_repair_group_sidecar_changed_keys` call per
/// [`KeyedSeedDiff`] descriptor, folded across every consumer of the same
/// upstream via [`fold_keyed_seed_values`] (the union rule). Read-only: this
/// never refreshes any sidecar partition — the refresh stays inside the
/// write transaction the run-time mechanism drives
/// ([`crate::maintenance_driver::execute_key_addressed_model_edge_cell`]), so
/// the live run's own diff re-derives the same set and the restriction union
/// at dispatch (`resolve_key_addressed_affected_keys`) only ever widens.
///
/// Per-model target overrides are not consulted here (a [`KeyedSeedDiff`]
/// carries bare addresses/db names only, not each model's own
/// `ModelMetadata`) — `crate::execute::model_edge_source_identity` resolves
/// against `config`'s `smelt.yml`-level `models:` overrides via the model
/// address, but not a frontmatter `target:` override. A workspace relying on
/// a per-model frontmatter target override for a keyed-edge participant is
/// unaffected in the common single-target case and is the one known gap of
/// this phase's live channel.
pub async fn resolve_keyed_seeds(
    backend: &dyn Backend,
    config: &smelt_core::config::Config,
    target: &str,
    diffs: &[KeyedSeedDiff],
) -> Result<BTreeMap<String, KeyValues>> {
    let mut per_upstream: BTreeMap<String, Vec<KeyValues>> = BTreeMap::new();
    for diff in diffs {
        let (upstream_source_address, upstream_table) = crate::execute::model_edge_source_identity(
            config,
            target,
            &diff.upstream,
            None,
            &diff.upstream_db_name,
        );
        let (_, consumer_table) = crate::execute::model_edge_source_identity(
            config,
            target,
            &diff.consumer,
            None,
            &diff.consumer_db_name,
        );
        let consumer_target = config.get_target(&diff.consumer, None, target);
        let consumer_schema = &config
            .targets
            .get(&consumer_target)
            .ok_or_else(|| anyhow::anyhow!("target '{consumer_target}' not found in smelt.yml"))?
            .schema;

        let result = diff_repair_group_sidecar_changed_keys(
            backend,
            consumer_schema,
            &upstream_source_address,
            &upstream_table,
            &consumer_table,
            &diff.upstream_keys,
            &diff.digest_columns,
            &diff.consumer_clean_sql,
        )
        .await;
        let key_values = keyed_seed_diff_result_to_key_values(result)?;
        per_upstream
            .entry(diff.upstream.clone())
            .or_default()
            .push(key_values);
    }
    Ok(per_upstream
        .into_iter()
        .map(|(upstream, values)| (upstream, fold_keyed_seed_values(values)))
        .collect())
}

/// The single owner of `--since-upstream`'s live-plan sequence
/// (`docs/outcomes/20260816-scheduler-delta-signatures/phases/12-plan.md`):
/// derive which `(model, window)` observed-delta keys and which `(upstream,
/// consumer)` keyed-seed descriptors matter (pure), read both live off
/// `backend`, then fold everything into a [`SinceUpstreamPlan`] via
/// [`plan_since_upstream_live`]. `run.rs::run_since_upstream` delegates here
/// so the generative conformance suite can drive the SAME sequence instead
/// of hand-rolling a copy that could silently drift from the real CLI path.
pub async fn resolve_live_plan(
    backend: &dyn Backend,
    config: &smelt_core::config::Config,
    target: &str,
    models: &[ModelFile],
    source_infos: &[SourceInfo],
    order: &[String],
    deltas: &[SourceDelta],
) -> Result<SinceUpstreamPlan> {
    let observed_keys = observed_delta_keys_to_read(models, source_infos, deltas)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let keyed_seed_diffs = keyed_seed_diffs_to_read(models, source_infos, deltas)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let schema = &config
        .targets
        .get(target)
        .ok_or_else(|| anyhow::anyhow!("target '{target}' not found in smelt.yml"))?
        .schema;
    let observed = resolve_observed_delta_lookup(backend, schema, &observed_keys).await?;
    let keyed_seeds = resolve_keyed_seeds(backend, config, target, &keyed_seed_diffs).await?;
    plan_since_upstream_live(models, source_infos, order, deltas, &observed, &keyed_seeds)
        .map_err(|e| anyhow::anyhow!("{e}"))
}
