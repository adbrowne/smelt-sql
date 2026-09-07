//! Live dispatch of the source append-only posture probe
//! (`docs/specs/model_properties.md` §"Probe obligation", row
//! `mutation_profile.kind: append_only`) for a succession-patch run, before
//! its fold — on the same terms as the ordinary `plan.incremental`
//! dispatch sites in `crate::execute::project` (`docs/outcomes/
//! 20260906-scd2-keyed-succession/phases/06c-plan.md`). Lifted verbatim
//! from those sites (never re-derived) so a late append into a closed
//! partition is tolerated and a genuine in-place mutation fails the run
//! loud before either the presented table or the tombstone ledger is
//! touched.

use anyhow::Result;

use smelt_backend::{Backend, MaintenanceDialect};
use smelt_core::sources::SourceInfo;
use smelt_core::ModelFile;
use smelt_state::file_store::FileStore;
use smelt_state::ProbeRecord;

use crate::probes::ProbePolicy;

/// Dispatch the declared append-only posture probes for `model_file`'s
/// consumed sources, refreshing (or establishing) each source's recorded
/// baseline in `file_store` under `state_io_lock` on a held/established
/// dispatch. Returns the accumulated [`ProbeRecord`]s for the caller's
/// `ModelRunRecord.probes`. Errors loud (before any write) on the first
/// `SourceMutationProfileViolated`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn dispatch_succession_source_probes(
    backend: &dyn Backend,
    policy: &ProbePolicy,
    file_store: &FileStore,
    state_io_lock: &tokio::sync::Mutex<()>,
    model_name: &str,
    cell_label: &str,
    model_file: &ModelFile,
    source_infos: &[SourceInfo],
    model_target: &str,
    schema: &str,
    dialect: MaintenanceDialect,
) -> Result<Vec<ProbeRecord>> {
    let mut model_probe_records = Vec::new();
    let _io_guard = state_io_lock.lock().await;
    let source_postures = file_store
        .load_source_postures()
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    let source_probes = crate::source_probes::append_only_posture_probes(
        model_name,
        cell_label,
        model_file,
        source_infos,
        &source_postures,
        model_target,
        schema,
        dialect,
    );
    if !source_probes.is_empty() {
        let (refreshed, records) = crate::source_probes::dispatch_and_record_append_only_postures(
            backend,
            policy,
            &source_probes,
        )
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;
        model_probe_records.extend(records);
        if !refreshed.is_empty() {
            let mut source_postures = source_postures;
            for r in refreshed {
                source_postures.record(&r.source_address, r.partitions);
            }
            let _ = file_store.save_source_postures(&source_postures);
        }
    }
    Ok(model_probe_records)
}
