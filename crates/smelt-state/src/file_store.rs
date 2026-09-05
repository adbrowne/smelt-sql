use crate::frozen_band_baselines::FrozenBandBaselineStore;
use crate::intervals::IntervalStore;
use crate::landed_deltas::LandedDeltaStore;
use crate::schema_tracking::DeployedSchema;
use crate::snapshot_store::SnapshotStore;
use crate::source_mutations::SourceMutationStore;
use crate::source_postures::SourcePostureStore;
use crate::RunManifest;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use smelt_core::config::StateMode;
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write as _};
use std::path::{Path, PathBuf};
use tracing::warn;

/// A distinct `.smelt/` structure `FileStore` reads or writes
/// (`docs/specs/state.md` §"The state-structure inventory", observability
/// rows only — correctness structures never go through `FileStore`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateArtifact {
    RunManifest,
    RunReport,
    Intervals,
    LandedDeltas,
    SourcePostures,
    SourceMutations,
    MigrationApprovals,
    FrozenBandBaselines,
    SchemaSnapshot,
    SnapshotStore,
}

/// The exact set of [`StateArtifact`]s a `state.mode` posture writes
/// (`docs/specs/state.md` §"`state.mode` and what each posture provides").
/// Pure data — the single table the posture gate in [`FileStore`] consults;
/// a future artifact is added here once, not at each save/load call site.
pub fn state_artifacts_written(mode: StateMode) -> &'static [StateArtifact] {
    use StateArtifact::*;
    const INTERVALS: [StateArtifact; 9] = [
        RunManifest,
        RunReport,
        Intervals,
        LandedDeltas,
        SourcePostures,
        SourceMutations,
        MigrationApprovals,
        FrozenBandBaselines,
        SchemaSnapshot,
    ];
    const ENVIRONMENTS: [StateArtifact; 10] = [
        RunManifest,
        RunReport,
        Intervals,
        LandedDeltas,
        SourcePostures,
        SourceMutations,
        MigrationApprovals,
        FrozenBandBaselines,
        SchemaSnapshot,
        SnapshotStore,
    ];
    match mode {
        StateMode::Stateless => &[],
        StateMode::Intervals => &INTERVALS,
        StateMode::Environments => &ENVIRONMENTS,
    }
}

/// The on-disk `.smelt/` layout version this binary writes and the highest
/// version it can read (`docs/specs/run_state.md` §"`meta.json` and layout
/// versioning"). Bump this — and add an explicit migration rule, never a
/// generic version-diff engine — the next time the layout changes.
///
/// v2 (`docs/plans/20260719-prod-w2-operability.md` Phase 7): run manifests
/// (`runs/<run_id>.json`) gained `outcome` and `definition_hash` per model,
/// needed for `--resume`. No migration is required on read: both fields are
/// `#[serde(default)]` on [`crate::ModelRunRecord`], so a v1-written
/// manifest still parses — `outcome` defaults to `Success` (accurate, since
/// a pre-v2 writer only ever persisted completed successes or explicit
/// check-skips) and `definition_hash` defaults to an empty string (which
/// never matches a real hash, so `--resume` always re-runs a model whose
/// only history predates this field — the safe direction).
pub const CURRENT_STATE_VERSION: u32 = 2;

/// `.smelt/meta.json` — the layout-version marker. A missing file denotes
/// the legacy pre-versioning root-level layout; a version greater than
/// [`CURRENT_STATE_VERSION`] is a hard error (never a silent best-effort
/// read).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StateMeta {
    state_version: u32,
}

/// Write `value` to `path` atomically: serialize, write to a `.tmp` sibling
/// in the same directory, `fsync`, then rename into place. A process killed
/// mid-write leaves either the old file intact or the new one — never a
/// truncated or partially-written file (`docs/specs/run_state.md`
/// §"Atomic writes").
fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {:?}", parent))?;
    }
    let json = serde_json::to_string_pretty(value)
        .with_context(|| format!("Failed to serialize state for {:?}", path))?;

    let mut tmp_name = path.as_os_str().to_os_string();
    tmp_name.push(".tmp");
    let tmp_path = PathBuf::from(tmp_name);

    let mut file = File::create(&tmp_path)
        .with_context(|| format!("Failed to create temp file: {:?}", tmp_path))?;
    file.write_all(json.as_bytes())
        .with_context(|| format!("Failed to write temp file: {:?}", tmp_path))?;
    file.sync_all()
        .with_context(|| format!("Failed to fsync temp file: {:?}", tmp_path))?;
    drop(file);

    std::fs::rename(&tmp_path, path)
        .with_context(|| format!("Failed to rename {:?} to {:?}", tmp_path, path))?;
    Ok(())
}

/// RAII guard for the exclusive advisory lock on `.smelt/lock`
/// (`docs/specs/run_state.md` §"Locking"). Held for a run's duration;
/// dropping the guard (on success or on any error path, since it is an
/// ordinary local binding) releases the lock.
#[derive(Debug)]
pub struct StateLock {
    /// `None` under a posture that writes no artifact (`state.mode:
    /// stateless`): `lock()` touched no path, so there is nothing to
    /// unlock on drop.
    file: Option<File>,
}

impl Drop for StateLock {
    fn drop(&mut self) {
        if let Some(file) = &self.file {
            let _ = fs4::FileExt::unlock(file);
        }
    }
}

/// JSON file-backed state store.
///
/// Stores data under `.smelt/` within the project directory, partitioned by
/// target (`docs/specs/run_state.md` §"`.smelt/` directory layout") so that
/// state for one target (e.g. `dev`) can never contaminate another (e.g.
/// `prod`):
/// - `.smelt/meta.json` — layout-version marker, project-wide
/// - `.smelt/lock` — advisory single-writer lock, project-wide (see below)
/// - `.smelt/targets/<target>/runs/{run_id}.json` — run manifests
/// - `.smelt/targets/<target>/intervals.json` — interval tracking
/// - … the remaining per-target artifacts listed in the spec.
///
/// **Why the lock is project-wide, not per-target.** A per-target lock
/// would let two processes write concurrently as long as they targeted
/// different environments, which sounds appealing but is wrong here: the
/// legacy-layout migration and the `meta.json` version stamp are project-wide
/// operations that touch `.smelt/` itself, not any one target's subtree, so
/// they need a lock that serializes against *every* writer, not just
/// same-target ones. A single project-global `.smelt/lock` is also simply
/// simpler — one lock file, one acquisition path — and the spec's stated
/// priority ("never lie" over throughput) favors the more conservative
/// choice. Nothing in the spec requires cross-target concurrent writes, so
/// there is no throughput cost being left on the table.
pub struct FileStore {
    /// `.smelt/` project root. Houses `meta.json` and `lock`, shared across
    /// every target.
    root_dir: PathBuf,
    /// `.smelt/targets/<target>/`. Houses every run-scoped artifact for
    /// this store's target.
    target_dir: PathBuf,
    /// The target this store was constructed for. Only consulted by the
    /// legacy-layout migration, which needs to know which target's
    /// subtree legacy root-level artifacts move into.
    target: String,
    /// The posture gate: `None` is the permissive read/tooling posture
    /// (every artifact allowed — [`FileStore::new`]'s long-standing
    /// behaviour, used by `smelt history`/`status`/`diff`/`migrate` and by
    /// every test that predates `state.mode` gating). `Some(mode)` gates
    /// every save/load to [`state_artifacts_written`]`(mode)`
    /// ([`FileStore::with_state_mode`], used by the run pipeline).
    mode: Option<StateMode>,
}

impl FileStore {
    /// Create a new FileStore rooted at `.smelt/targets/<target>/` under the
    /// given project directory. `meta.json` and `lock` remain project-wide
    /// at `.smelt/` regardless of `target`. Permissive: every artifact may
    /// be read or written regardless of any project's `state.mode` — the
    /// constructor for read/tooling paths (history, status, diff, migrate)
    /// that must see whatever a run actually wrote, not what an unrelated
    /// posture would have allowed.
    pub fn new(project_dir: &Path, target: &str) -> Self {
        let root_dir = project_dir.join(".smelt");
        Self {
            target_dir: root_dir.join("targets").join(target),
            root_dir,
            target: target.to_string(),
            mode: None,
        }
    }

    /// Create a `FileStore` gated to exactly the artifacts `mode` writes
    /// (`docs/specs/state.md` §"`state.mode` and what each posture
    /// provides"). The run pipeline's constructor: every `save_*`/`load_*`/
    /// `init`/`lock` call is a no-op (writes touch no path; reads return
    /// the default) for an artifact `mode` excludes, so a `stateless`
    /// project's run never creates `.smelt/` at all.
    pub fn with_state_mode(project_dir: &Path, target: &str, mode: StateMode) -> Self {
        Self {
            mode: Some(mode),
            ..Self::new(project_dir, target)
        }
    }

    /// Whether `artifact` is writable/readable under this store's posture.
    fn allows(&self, artifact: StateArtifact) -> bool {
        match self.mode {
            None => true,
            Some(mode) => state_artifacts_written(mode).contains(&artifact),
        }
    }

    /// Whether this store's posture writes any artifact at all — `lock()`
    /// and `init()` no-op entirely when this is false, since a posture
    /// that writes nothing must not create `.smelt/` itself.
    fn allows_any(&self) -> bool {
        match self.mode {
            None => true,
            Some(mode) => !state_artifacts_written(mode).is_empty(),
        }
    }

    /// Ensure the state directories exist. A no-op under a posture that
    /// writes no artifact at all (`state.mode: stateless`) — never creates
    /// `.smelt/` on a project that opted out of it.
    pub fn init(&self) -> Result<()> {
        if !self.allows_any() {
            return Ok(());
        }
        self.check_version()?;
        std::fs::create_dir_all(self.runs_dir())
            .with_context(|| format!("Failed to create runs directory: {:?}", self.runs_dir()))?;
        Ok(())
    }

    fn runs_dir(&self) -> PathBuf {
        self.target_dir.join("runs")
    }

    fn reports_dir(&self) -> PathBuf {
        self.target_dir.join("reports")
    }

    fn intervals_path(&self) -> PathBuf {
        self.target_dir.join("intervals.json")
    }

    fn landed_deltas_path(&self) -> PathBuf {
        self.target_dir.join("landed_deltas.json")
    }

    fn source_postures_path(&self) -> PathBuf {
        self.target_dir.join("source_postures.json")
    }

    fn source_mutations_path(&self) -> PathBuf {
        self.target_dir.join("source_mutations.json")
    }

    fn migration_approvals_path(&self) -> PathBuf {
        self.target_dir.join("migration_approvals.json")
    }

    fn frozen_band_baselines_path(&self) -> PathBuf {
        self.target_dir.join("frozen_band_baselines.json")
    }

    fn snapshots_path(&self) -> PathBuf {
        self.target_dir.join("snapshots.json")
    }

    fn schemas_dir(&self) -> PathBuf {
        self.target_dir.join("schemas")
    }

    fn lock_path(&self) -> PathBuf {
        self.root_dir.join("lock")
    }

    fn meta_path(&self) -> PathBuf {
        self.root_dir.join("meta.json")
    }

    /// Hard-error if `.smelt/meta.json` records a layout version newer than
    /// this binary understands. A missing `meta.json` is the legacy
    /// pre-versioning layout and is not an error here — only [`lock`]
    /// performs the one-time upgrade write, so a version check outside the
    /// lock never itself mutates `.smelt/`.
    ///
    /// Called by every read/write entry point (`init`, and each `load_*`
    /// method) so an unrecognised future version is refused loudly no
    /// matter which operation is attempted first
    /// (`docs/specs/run_state.md` §"Layout version is checked before any
    /// read or write").
    fn check_version(&self) -> Result<()> {
        let path = self.meta_path();
        if !path.exists() {
            return Ok(());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read state layout marker: {:?}", path))?;
        let meta: StateMeta = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse state layout marker: {:?}", path))?;
        if meta.state_version > CURRENT_STATE_VERSION {
            anyhow::bail!(
                "'.smelt/' was written with state_version {} but this smelt binary only understands up to version {}. Refusing to read or write .smelt/ — upgrade smelt, or remove .smelt/ to start fresh.",
                meta.state_version,
                CURRENT_STATE_VERSION
            );
        }
        Ok(())
    }

    /// Validate the on-disk layout version, upgrading a missing
    /// `meta.json` (the legacy pre-versioning layout) by migrating any
    /// root-level artifacts into `targets/<target>/` and then stamping the
    /// current version. Must run only while holding the exclusive lock
    /// (`lock()` calls this) so a concurrent second process never observes
    /// a half-migrated layout (`docs/specs/run_state.md` §"Locking").
    fn check_and_upgrade_meta_locked(&self) -> Result<()> {
        let path = self.meta_path();
        if !path.exists() {
            self.migrate_legacy_layout_locked()?;
            return write_json_atomic(
                &path,
                &StateMeta {
                    state_version: CURRENT_STATE_VERSION,
                },
            );
        }
        self.check_version()
    }

    /// Move any legacy root-level state artifacts into
    /// `targets/<target>/` for this store's target
    /// (`docs/specs/run_state.md` §"`meta.json` and layout versioning": "the
    /// first locked open of `.smelt/` … migrates a legacy layout … for the
    /// target of the run doing the migration"). Idempotent: called only from
    /// [`check_and_upgrade_meta_locked`] before `meta.json` is written, and
    /// each item is moved via `rename` which leaves nothing behind at the
    /// source — a second call (which can only happen if a prior attempt
    /// failed before the `meta.json` write) finds no source files left to
    /// move and is a no-op.
    ///
    /// The spec's legacy-layout list names `runs/`, `intervals.json`,
    /// `landed_deltas.json`, `schemas/`; this also moves `snapshots.json`
    /// even though the spec prose omits it, since it is a current per-target
    /// artifact kind that predates this migration and leaving it stranded
    /// at the project root would silently orphan it — failing loud here
    /// would mean silently *not* migrating it, which is the wrong kind of
    /// fail-loud. `reconciliation.json` is deliberately absent: the
    /// reconciliation ledger is engine-resident now (`_smelt_ledger`,
    /// `docs/outcomes/20260904-state-residency/outcome.md`), so a legacy
    /// root-level `reconciliation.json` from a pre-residency `.smelt/` is
    /// inert dead weight — left in place, not migrated.
    fn migrate_legacy_layout_locked(&self) -> Result<()> {
        std::fs::create_dir_all(&self.target_dir).with_context(|| {
            format!(
                "Failed to create target state directory: {:?}",
                self.target_dir
            )
        })?;
        for name in [
            "runs",
            "intervals.json",
            "landed_deltas.json",
            "snapshots.json",
            "schemas",
        ] {
            let src = self.root_dir.join(name);
            if !src.exists() {
                continue;
            }
            let dst = self.target_dir.join(name);
            std::fs::rename(&src, &dst).with_context(|| {
                format!(
                    "Failed to migrate legacy state '{}' from {:?} to {:?} (target {:?})",
                    name, src, dst, self.target
                )
            })?;
        }
        Ok(())
    }

    /// Acquire the exclusive advisory lock on `.smelt/lock` for the
    /// duration of a run. A second process contending for the lock fails
    /// loudly, naming the holder's PID, rather than interleaving writes
    /// (`docs/specs/run_state.md` §"Locking"). Also performs the one-time
    /// legacy-layout `meta.json` upgrade / future-version hard-error check
    /// under the lock.
    pub fn lock(&self) -> Result<StateLock> {
        if !self.allows_any() {
            return Ok(StateLock { file: None });
        }
        std::fs::create_dir_all(&self.root_dir)
            .with_context(|| format!("Failed to create state directory: {:?}", self.root_dir))?;
        let lock_path = self.lock_path();
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("Failed to open lock file: {:?}", lock_path))?;

        if let Err(err) = fs4::FileExt::try_lock(&file) {
            return match err {
                fs4::TryLockError::WouldBlock => {
                    let holder = std::fs::read_to_string(&lock_path).unwrap_or_default();
                    let holder = holder.trim();
                    anyhow::bail!("state locked by PID {}", holder)
                }
                fs4::TryLockError::Error(io_err) => {
                    Err(io_err).with_context(|| format!("Failed to lock {:?}", lock_path))
                }
            };
        }

        // We now hold the exclusive lock: record our own PID so a
        // contending process can name us in its error, then validate/
        // upgrade the layout version marker under the lock.
        file.set_len(0)
            .with_context(|| format!("Failed to truncate lock file: {:?}", lock_path))?;
        file.seek(SeekFrom::Start(0))
            .with_context(|| format!("Failed to seek lock file: {:?}", lock_path))?;
        write!(file, "{}", std::process::id())
            .with_context(|| format!("Failed to write PID to lock file: {:?}", lock_path))?;
        file.sync_all().ok();

        if let Err(err) = self.check_and_upgrade_meta_locked() {
            let _ = fs4::FileExt::unlock(&file);
            return Err(err);
        }

        Ok(StateLock { file: Some(file) })
    }

    // --- Run Manifests ---

    /// Save a run manifest to disk.
    pub fn save_run(&self, manifest: &RunManifest) -> Result<()> {
        if !self.allows(StateArtifact::RunManifest) {
            return Ok(());
        }
        self.init()?;
        let path = self.runs_dir().join(format!("{}.json", manifest.run_id));
        write_json_atomic(&path, manifest)
            .with_context(|| format!("Failed to write run manifest: {:?}", path))
    }

    /// Load run manifests, sorted by run_id (newest first).
    ///
    /// If `limit` is `Some(n)`, only the most recent `n` manifests are returned.
    /// Files are sorted by name (descending) before loading, so with a limit
    /// only the newest files are read from disk.
    pub fn load_runs(&self, limit: Option<usize>) -> Result<Vec<RunManifest>> {
        if !self.allows(StateArtifact::RunManifest) {
            return Ok(Vec::new());
        }
        self.check_version()?;
        let runs_dir = self.runs_dir();
        if !runs_dir.exists() {
            return Ok(Vec::new());
        }

        let mut paths: Vec<_> = std::fs::read_dir(&runs_dir)
            .with_context(|| format!("Failed to read runs directory: {:?}", runs_dir))?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect();

        // Sort descending by filename (run_id encodes timestamp, so newest first)
        paths.sort_by(|a, b| b.cmp(a));

        if let Some(n) = limit {
            paths.truncate(n);
        }

        let mut manifests = Vec::new();
        for path in &paths {
            match std::fs::read_to_string(path) {
                Ok(content) => match serde_json::from_str::<RunManifest>(&content) {
                    Ok(manifest) => manifests.push(manifest),
                    Err(e) => {
                        warn!("failed to parse run manifest {:?}: {}", path, e);
                    }
                },
                Err(e) => {
                    warn!("failed to read {:?}: {}", path, e);
                }
            }
        }

        // Already sorted by filename, but re-sort by run_id for correctness
        manifests.sort_by(|a, b| b.run_id.cmp(&a.run_id));
        Ok(manifests)
    }

    /// Load a specific run manifest by ID.
    pub fn load_run(&self, run_id: &str) -> Result<Option<RunManifest>> {
        if !self.allows(StateArtifact::RunManifest) {
            return Ok(None);
        }
        self.check_version()?;
        let path = self.runs_dir().join(format!("{}.json", run_id));
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read run manifest: {:?}", path))?;
        let manifest = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse run manifest: {:?}", path))?;
        Ok(Some(manifest))
    }

    // --- Run Reports ---

    /// Save a run report to disk, alongside its manifest
    /// (`docs/specs/run_state.md` §"Run report").
    pub fn save_report(&self, report: &crate::RunReport) -> Result<()> {
        if !self.allows(StateArtifact::RunReport) {
            return Ok(());
        }
        self.init()?;
        let dir = self.reports_dir();
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("Failed to create reports directory: {:?}", dir))?;
        let path = dir.join(format!("{}.json", report.run_id));
        write_json_atomic(&path, report)
            .with_context(|| format!("Failed to write run report: {:?}", path))
    }

    /// Load a specific run report by ID.
    pub fn load_report(&self, run_id: &str) -> Result<Option<crate::RunReport>> {
        if !self.allows(StateArtifact::RunReport) {
            return Ok(None);
        }
        self.check_version()?;
        let path = self.reports_dir().join(format!("{}.json", run_id));
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read run report: {:?}", path))?;
        let report = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse run report: {:?}", path))?;
        Ok(Some(report))
    }

    // --- Interval Store ---

    /// Load the interval store from disk. Returns default if file doesn't exist.
    pub fn load_intervals(&self) -> Result<IntervalStore> {
        if !self.allows(StateArtifact::Intervals) {
            return Ok(IntervalStore::default());
        }
        self.check_version()?;
        let path = self.intervals_path();
        if !path.exists() {
            return Ok(IntervalStore::default());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read intervals: {:?}", path))?;
        let store = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse intervals: {:?}", path))?;
        Ok(store)
    }

    /// Save the interval store to disk.
    pub fn save_intervals(&self, store: &IntervalStore) -> Result<()> {
        if !self.allows(StateArtifact::Intervals) {
            return Ok(());
        }
        self.init()?;
        let path = self.intervals_path();
        write_json_atomic(&path, store)
            .with_context(|| format!("Failed to write intervals: {:?}", path))
    }

    // --- Landed-delta store ---

    /// Load the per-source landed-delta store from disk (`docs/specs/sources.md`
    /// §"World-facts admission consumes"). Returns default if the file
    /// doesn't exist — a source with no entry has never had a landing
    /// recorded.
    pub fn load_landed_deltas(&self) -> Result<LandedDeltaStore> {
        if !self.allows(StateArtifact::LandedDeltas) {
            return Ok(LandedDeltaStore::default());
        }
        self.check_version()?;
        let path = self.landed_deltas_path();
        if !path.exists() {
            return Ok(LandedDeltaStore::default());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read landed-delta store: {:?}", path))?;
        let store = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse landed-delta store: {:?}", path))?;
        Ok(store)
    }

    /// Save the per-source landed-delta store to disk.
    pub fn save_landed_deltas(&self, store: &LandedDeltaStore) -> Result<()> {
        if !self.allows(StateArtifact::LandedDeltas) {
            return Ok(());
        }
        self.init()?;
        let path = self.landed_deltas_path();
        write_json_atomic(&path, store)
            .with_context(|| format!("Failed to write landed-delta store: {:?}", path))
    }

    // --- Source posture store ---

    /// Load the per-source append-only posture baseline store from disk
    /// (`docs/specs/model_properties.md` §"Probe obligation", row
    /// `mutation_profile.kind: append_only`). Returns default if the file
    /// doesn't exist — a source with no entry has never had its posture
    /// verified, so builds no probe.
    pub fn load_source_postures(&self) -> Result<SourcePostureStore> {
        if !self.allows(StateArtifact::SourcePostures) {
            return Ok(SourcePostureStore::default());
        }
        self.check_version()?;
        let path = self.source_postures_path();
        if !path.exists() {
            return Ok(SourcePostureStore::default());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read source-posture store: {:?}", path))?;
        let store = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse source-posture store: {:?}", path))?;
        Ok(store)
    }

    /// Save the per-source append-only posture baseline store to disk.
    pub fn save_source_postures(&self, store: &SourcePostureStore) -> Result<()> {
        if !self.allows(StateArtifact::SourcePostures) {
            return Ok(());
        }
        self.init()?;
        let path = self.source_postures_path();
        write_json_atomic(&path, store)
            .with_context(|| format!("Failed to write source-posture store: {:?}", path))
    }

    // --- Source mutation store ---

    /// Load the per-source mutation-fingerprint baseline store from disk
    /// (`docs/specs/incremental_models.md` §"When a mutation cell
    /// dispatches"). Returns default if the file doesn't exist — a source
    /// with no entry has never had a dispatched `UpstreamMutation` cell
    /// record a baseline, so its cell unconditionally dispatches.
    pub fn load_source_mutations(&self) -> Result<SourceMutationStore> {
        if !self.allows(StateArtifact::SourceMutations) {
            return Ok(SourceMutationStore::default());
        }
        self.check_version()?;
        let path = self.source_mutations_path();
        if !path.exists() {
            return Ok(SourceMutationStore::default());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read source-mutation store: {:?}", path))?;
        let store = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse source-mutation store: {:?}", path))?;
        Ok(store)
    }

    /// Save the per-source mutation-fingerprint baseline store to disk.
    pub fn save_source_mutations(&self, store: &SourceMutationStore) -> Result<()> {
        if !self.allows(StateArtifact::SourceMutations) {
            return Ok(());
        }
        self.init()?;
        let path = self.source_mutations_path();
        write_json_atomic(&path, store)
            .with_context(|| format!("Failed to write source-mutation store: {:?}", path))
    }

    // --- Migration approval store ---

    /// Load the per-model migration-plan approval store from disk
    /// (`docs/specs/definition_deltas.md` §"`smelt migrate`"). Returns
    /// default if the file doesn't exist — a model with no entry has never
    /// had a migration plan derived and printed for approval.
    pub fn load_migration_approvals(
        &self,
    ) -> Result<crate::migration_approvals::MigrationApprovalStore> {
        if !self.allows(StateArtifact::MigrationApprovals) {
            return Ok(crate::migration_approvals::MigrationApprovalStore::default());
        }
        self.check_version()?;
        let path = self.migration_approvals_path();
        if !path.exists() {
            return Ok(crate::migration_approvals::MigrationApprovalStore::default());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read migration-approval store: {:?}", path))?;
        let store = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse migration-approval store: {:?}", path))?;
        Ok(store)
    }

    /// Save the per-model migration-plan approval store to disk.
    pub fn save_migration_approvals(
        &self,
        store: &crate::migration_approvals::MigrationApprovalStore,
    ) -> Result<()> {
        if !self.allows(StateArtifact::MigrationApprovals) {
            return Ok(());
        }
        self.init()?;
        let path = self.migration_approvals_path();
        write_json_atomic(&path, store)
            .with_context(|| format!("Failed to write migration-approval store: {:?}", path))
    }

    // --- Contract-lattice frozen-band baseline store ---

    /// Load the per-source frozen-band row-count baseline store from disk
    /// (`docs/specs/incremental_models.md` §"The contract lattice", frozen
    /// horizon). Returns default if the file doesn't exist — a source with
    /// no entry has never had its frozen band snapshotted, so its next
    /// observation is unconditionally established.
    pub fn load_frozen_band_baselines(&self) -> Result<FrozenBandBaselineStore> {
        if !self.allows(StateArtifact::FrozenBandBaselines) {
            return Ok(FrozenBandBaselineStore::default());
        }
        self.check_version()?;
        let path = self.frozen_band_baselines_path();
        if !path.exists() {
            return Ok(FrozenBandBaselineStore::default());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read frozen-band baseline store: {:?}", path))?;
        let store = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse frozen-band baseline store: {:?}", path))?;
        Ok(store)
    }

    /// Save the per-source frozen-band row-count baseline store to disk.
    pub fn save_frozen_band_baselines(&self, store: &FrozenBandBaselineStore) -> Result<()> {
        if !self.allows(StateArtifact::FrozenBandBaselines) {
            return Ok(());
        }
        self.init()?;
        let path = self.frozen_band_baselines_path();
        write_json_atomic(&path, store)
            .with_context(|| format!("Failed to write frozen-band baseline store: {:?}", path))
    }

    // --- Snapshot / Environment Store ---

    /// Load the snapshot store from disk. Returns an empty store if the file doesn't exist.
    pub fn load_snapshot_store(&self) -> Result<SnapshotStore> {
        if !self.allows(StateArtifact::SnapshotStore) {
            return Ok(SnapshotStore::default());
        }
        self.check_version()?;
        let path = self.snapshots_path();
        if !path.exists() {
            return Ok(SnapshotStore::default());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read snapshot store: {:?}", path))?;
        let store = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse snapshot store: {:?}", path))?;
        Ok(store)
    }

    /// Save the snapshot store to disk.
    pub fn save_snapshot_store(&self, store: &SnapshotStore) -> Result<()> {
        if !self.allows(StateArtifact::SnapshotStore) {
            return Ok(());
        }
        self.init()?;
        let path = self.snapshots_path();
        write_json_atomic(&path, store)
            .with_context(|| format!("Failed to write snapshot store: {:?}", path))
    }

    // --- Schema Tracking ---

    /// Save a deployed schema for a model.
    pub fn save_schema(&self, schema: &DeployedSchema) -> Result<()> {
        if !self.allows(StateArtifact::SchemaSnapshot) {
            return Ok(());
        }
        self.check_version()?;
        let dir = self.schemas_dir();
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("Failed to create schemas directory: {:?}", dir))?;
        let path = dir.join(format!("{}.json", schema.model));
        write_json_atomic(&path, schema)
            .with_context(|| format!("Failed to write schema: {:?}", path))
    }

    /// Load the deployed schema for a model. Returns None if not found.
    pub fn load_schema(&self, model_name: &str) -> Result<Option<DeployedSchema>> {
        if !self.allows(StateArtifact::SchemaSnapshot) {
            return Ok(None);
        }
        self.check_version()?;
        let path = self.schemas_dir().join(format!("{}.json", model_name));
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read schema: {:?}", path))?;
        let schema = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse schema: {:?}", path))?;
        Ok(Some(schema))
    }

    /// List all model names that have deployed schemas.
    ///
    /// Returns the file stems from `.smelt/targets/<target>/schemas/*.json`.
    /// Returns an empty vec if the schemas directory doesn't exist.
    pub fn list_deployed_model_names(&self) -> Vec<String> {
        if !self.allows(StateArtifact::SchemaSnapshot) {
            return Vec::new();
        }
        let dir = self.schemas_dir();
        if !dir.exists() {
            return Vec::new();
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Vec::new();
        };
        entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "json") {
                    path.file_stem()
                        .and_then(|s| s.to_str())
                        .map(|s| s.to_string())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Delete the deployed schema for a model.
    ///
    /// No-ops silently if the file does not exist. Called by the build lifecycle
    /// after a successful run to remove orphan entries for deleted model files.
    pub fn delete_schema(&self, model_name: &str) -> Result<()> {
        if !self.allows(StateArtifact::SchemaSnapshot) {
            return Ok(());
        }
        let path = self.schemas_dir().join(format!("{}.json", model_name));
        if path.exists() {
            std::fs::remove_file(&path)
                .with_context(|| format!("Failed to delete schema: {:?}", path))?;
        }
        Ok(())
    }

    /// Check if this target's state directory exists (indicates state
    /// tracking has been initialized for this target). Does not report on
    /// other targets' state or on a not-yet-migrated legacy root layout —
    /// callers that need that need to check `root_dir` explicitly, which no
    /// current caller does.
    pub fn exists(&self) -> bool {
        self.target_dir.exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema_tracking::DeployedColumn;
    use crate::snapshot_store::SnapshotEntry;
    use crate::{ModelRunRecord, TimeRangeRecord};
    use chrono::Utc;
    use std::collections::HashMap;
    use tempfile::TempDir;

    fn test_manifest() -> RunManifest {
        let mut models = HashMap::new();
        models.insert(
            "daily_revenue".to_string(),
            ModelRunRecord {
                strategy: "delete_insert".to_string(),
                time_range: Some(TimeRangeRecord {
                    start: "2026-03-20".to_string(),
                    end: "2026-03-22".to_string(),
                }),
                partitions_updated: vec!["2026-03-20".to_string(), "2026-03-21".to_string()],
                row_count: 1542,
                duration_ms: 230,
                batch_safety: Some("fully_batch_safe".to_string()),
                outcome: crate::RunOutcomeKind::Success,
                definition_hash: "sha256:abc".to_string(),
                error: None,
                retry_count: 0,
                probes: Vec::new(),
                subsumed: None,
                deferred_cells: Vec::new(),
            },
        );
        RunManifest {
            run_id: "20260322-143022-abc123".to_string(),
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
            models,
        }
    }

    #[test]
    fn test_save_and_load_run() {
        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path(), "dev");

        let manifest = test_manifest();
        store.save_run(&manifest).unwrap();

        let loaded = store.load_run(&manifest.run_id).unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.run_id, manifest.run_id);
        assert!(loaded.models.contains_key("daily_revenue"));
    }

    #[test]
    fn test_load_all_runs() {
        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path(), "dev");

        let mut m1 = test_manifest();
        m1.run_id = "20260322-100000-aaa".to_string();
        let mut m2 = test_manifest();
        m2.run_id = "20260322-110000-bbb".to_string();

        store.save_run(&m1).unwrap();
        store.save_run(&m2).unwrap();

        let runs = store.load_runs(None).unwrap();
        assert_eq!(runs.len(), 2);
        // Newest first
        assert_eq!(runs[0].run_id, "20260322-110000-bbb");
    }

    #[test]
    fn test_intervals_roundtrip() {
        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path(), "dev");

        let mut intervals = IntervalStore::default();
        let model = intervals.get_or_create("daily_revenue", "sha256:abc");
        model.record_interval("2026-01-01", "2026-03-22");

        store.save_intervals(&intervals).unwrap();

        let loaded = store.load_intervals().unwrap();
        assert!(loaded.get("daily_revenue").is_some());
        assert_eq!(
            loaded.get("daily_revenue").unwrap().covered_intervals.len(),
            1
        );
    }

    #[test]
    fn test_landed_deltas_roundtrip() {
        use crate::landed_deltas::LandedDeltaStore;

        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path(), "dev");

        let mut deltas = LandedDeltaStore::default();
        let delta = deltas
            .get_or_create("sources.orders")
            .record_landing("2026-01-01", "2026-01-10");
        assert!(!delta.is_empty());

        store.save_landed_deltas(&deltas).unwrap();

        let loaded = store.load_landed_deltas().unwrap();
        assert!(loaded.get("sources.orders").is_some());
        assert_eq!(
            loaded
                .get("sources.orders")
                .unwrap()
                .covered_intervals
                .len(),
            1
        );
    }

    #[test]
    fn test_landed_deltas_empty_when_file_missing() {
        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path(), "dev");
        let loaded = store.load_landed_deltas().unwrap();
        assert!(loaded.sources.is_empty());
    }

    #[test]
    fn source_mutation_store_round_trips() {
        use crate::source_mutations::{SourceMutationBaseline, SourceMutationStore};

        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path(), "dev");

        // Unrecorded source has no baseline.
        let loaded = store.load_source_mutations().unwrap();
        assert!(loaded.get("sources.events").is_none());

        let mut mutations = SourceMutationStore::default();
        mutations.record(
            "sources.events",
            SourceMutationBaseline {
                recorded_count: 42,
                recorded_fingerprint: "abc123".to_string(),
                digest_columns: vec!["event_id".to_string()],
            },
        );
        store.save_source_mutations(&mutations).unwrap();

        let loaded = store.load_source_mutations().unwrap();
        let baseline = loaded.get("sources.events").unwrap();
        assert_eq!(baseline.recorded_count, 42);
        assert_eq!(baseline.recorded_fingerprint, "abc123");
        assert_eq!(baseline.digest_columns, vec!["event_id".to_string()]);
    }

    #[test]
    fn test_empty_store() {
        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path(), "dev");

        let runs = store.load_runs(None).unwrap();
        assert!(runs.is_empty());

        let intervals = store.load_intervals().unwrap();
        assert!(intervals.models.is_empty());
    }

    #[test]
    fn test_schema_save_and_load() {
        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path(), "dev");

        let schema = DeployedSchema {
            model: "daily_revenue".to_string(),
            version: 1,
            deployed_at: Utc::now(),
            model_hash: "sha256:abc".to_string(),
            model_sql: None,
            partition_column: None,
            columns: vec![
                DeployedColumn {
                    name: "order_date".to_string(),
                    data_type: "DATE".to_string(),
                    nullable: false,
                },
                DeployedColumn {
                    name: "total".to_string(),
                    data_type: "DECIMAL(10,2)".to_string(),
                    nullable: true,
                },
            ],
        };

        store.save_schema(&schema).unwrap();

        let loaded = store.load_schema("daily_revenue").unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.model, "daily_revenue");
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.columns.len(), 2);
    }

    #[test]
    fn test_schema_not_found() {
        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path(), "dev");

        let loaded = store.load_schema("nonexistent").unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn test_delete_schema_removes_file() {
        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path(), "dev");

        let schema = DeployedSchema {
            model: "stg_orders".to_string(),
            version: 1,
            deployed_at: Utc::now(),
            model_hash: "sha256:abc".to_string(),
            model_sql: None,
            partition_column: None,
            columns: vec![],
        };
        store.save_schema(&schema).unwrap();
        assert!(store.load_schema("stg_orders").unwrap().is_some());

        store.delete_schema("stg_orders").unwrap();
        assert!(store.load_schema("stg_orders").unwrap().is_none());

        // list_deployed_model_names should no longer include it
        let names = store.list_deployed_model_names();
        assert!(!names.contains(&"stg_orders".to_string()));
    }

    #[test]
    fn test_delete_schema_noop_when_missing() {
        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path(), "dev");

        // Deleting a non-existent schema should not error
        store.delete_schema("nonexistent").unwrap();
    }

    #[test]
    fn test_snapshot_store_roundtrip() {
        let dir = TempDir::new().unwrap();
        let file_store = FileStore::new(dir.path(), "dev");

        let mut snap = SnapshotStore::default();
        snap.upsert(SnapshotEntry {
            model: "orders".to_string(),
            environment: "prod".to_string(),
            physical_table: "orders__prod".to_string(),
            source_sql: "SELECT * FROM raw.orders".to_string(),
            fingerprint_hex: Some("fp_abc123".to_string()),
        });
        snap.upsert(SnapshotEntry {
            model: "customers".to_string(),
            environment: "dev".to_string(),
            physical_table: "customers__dev".to_string(),
            source_sql: "SELECT * FROM raw.customers".to_string(),
            fingerprint_hex: None,
        });

        file_store.save_snapshot_store(&snap).unwrap();

        let loaded = file_store.load_snapshot_store().unwrap();
        assert_eq!(loaded.len(), 2);

        let e = loaded.get("prod", "orders").unwrap();
        assert_eq!(e.physical_table, "orders__prod");
        assert_eq!(e.fingerprint_hex.as_deref(), Some("fp_abc123"));

        let e2 = loaded.get("dev", "customers").unwrap();
        assert!(e2.fingerprint_hex.is_none());
    }

    #[test]
    fn test_snapshot_store_empty_when_file_missing() {
        let dir = TempDir::new().unwrap();
        let file_store = FileStore::new(dir.path(), "dev");

        let loaded = file_store.load_snapshot_store().unwrap();
        assert!(loaded.is_empty());
    }

    // --- Phase 3: state store hardening ---

    #[test]
    fn atomic_write_leaves_no_temp_files() {
        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path(), "dev");

        let mut intervals = IntervalStore::default();
        intervals
            .get_or_create("daily_revenue", "sha256:abc")
            .record_interval("2026-01-01", "2026-01-02");
        store.save_intervals(&intervals).unwrap();

        // No stray .tmp files anywhere under `.smelt/` (root or per-target).
        fn collect_names(dir: &std::path::Path, out: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()) {
                let path = entry.path();
                out.push(entry.file_name().to_string_lossy().into_owned());
                if path.is_dir() {
                    collect_names(&path, out);
                }
            }
        }
        let mut entries = Vec::new();
        collect_names(&dir.path().join(".smelt"), &mut entries);
        assert!(
            !entries.iter().any(|name| name.ends_with(".tmp")),
            "expected no leftover .tmp files, got {entries:?}"
        );
        assert!(
            entries.contains(&"intervals.json".to_string()),
            "expected intervals.json to exist, got {entries:?}"
        );

        // Content round-trips.
        let loaded = store.load_intervals().unwrap();
        assert!(loaded.get("daily_revenue").is_some());
        assert_eq!(
            loaded.get("daily_revenue").unwrap().covered_intervals.len(),
            1
        );
    }

    #[test]
    fn second_lock_holder_gets_fail_loud_error() {
        let dir = TempDir::new().unwrap();
        let store1 = FileStore::new(dir.path(), "dev");
        let store2 = FileStore::new(dir.path(), "dev");

        let _guard1 = store1.lock().unwrap();
        let err = store2
            .lock()
            .expect_err("second lock acquisition must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("state locked by PID"),
            "unexpected error message: {msg}"
        );
        assert!(
            msg.contains(&std::process::id().to_string()),
            "expected the holder's PID in the error message: {msg}"
        );
    }

    #[test]
    fn future_state_version_is_hard_error() {
        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path(), "dev");
        store.init().unwrap();
        std::fs::write(
            dir.path().join(".smelt").join("meta.json"),
            r#"{"state_version": 99}"#,
        )
        .unwrap();

        let err = store
            .load_intervals()
            .expect_err("a future state_version must hard-error on load");
        assert!(
            err.to_string().contains("99"),
            "error should name the on-disk version: {err}"
        );

        let err2 = store
            .lock()
            .expect_err("a future state_version must hard-error on lock");
        assert!(
            err2.to_string().contains("99"),
            "error should name the on-disk version: {err2}"
        );
    }

    #[test]
    fn missing_meta_json_is_legacy_and_upgraded() {
        let dir = TempDir::new().unwrap();

        // Simulate a pre-versioning, legacy (root-level, non-per-target)
        // legacy layout by writing directly at `.smelt/intervals.json`,
        // bypassing FileStore entirely (FileStore always writes under
        // `targets/<target>/` now).
        let smelt_dir = dir.path().join(".smelt");
        std::fs::create_dir_all(&smelt_dir).unwrap();
        let mut intervals = IntervalStore::default();
        intervals
            .get_or_create("daily_revenue", "sha256:abc")
            .record_interval("2026-01-01", "2026-01-02");
        std::fs::write(
            smelt_dir.join("intervals.json"),
            serde_json::to_string_pretty(&intervals).unwrap(),
        )
        .unwrap();
        let meta_path = smelt_dir.join("meta.json");
        assert!(!meta_path.exists());

        let store = FileStore::new(dir.path(), "dev");

        // First locked open upgrades the layout by stamping the current
        // version and migrating the legacy root-level file under the
        // target that performed the migration.
        let _guard = store.lock().unwrap();
        assert!(meta_path.exists());
        let content = std::fs::read_to_string(&meta_path).unwrap();
        assert!(
            content.contains(&CURRENT_STATE_VERSION.to_string()),
            "expected meta.json to record the current state_version, got: {content}"
        );
        assert!(
            !smelt_dir.join("intervals.json").exists(),
            "legacy root-level intervals.json should have been moved, not left behind"
        );

        // Pre-existing state is still readable after the upgrade, now from
        // its migrated location.
        let loaded = store.load_intervals().unwrap();
        assert!(loaded.get("daily_revenue").is_some());
    }

    /// `docs/specs/run_state.md` §"`.smelt/` directory layout": every
    /// run-scoped artifact lives under `.smelt/targets/<target>/`, keyed by
    /// target, so a `dev` write can never leak into a `prod` read (or vice
    /// versa).
    #[test]
    fn stores_for_different_targets_are_disjoint() {
        let dir = TempDir::new().unwrap();
        let dev_store = FileStore::new(dir.path(), "dev");
        let prod_store = FileStore::new(dir.path(), "prod");

        let mut dev_intervals = IntervalStore::default();
        dev_intervals
            .get_or_create("daily_revenue", "sha256:abc")
            .record_interval("2026-01-01", "2026-01-02");
        dev_store.save_intervals(&dev_intervals).unwrap();

        // The prod store sees no intervals at all — the dev write is
        // invisible to it.
        let prod_intervals = prod_store.load_intervals().unwrap();
        assert!(
            prod_intervals.get("daily_revenue").is_none(),
            "prod target must not see dev target's interval writes"
        );

        // And a prod-side write doesn't perturb dev's view.
        let mut prod_intervals_to_write = IntervalStore::default();
        prod_intervals_to_write
            .get_or_create("daily_revenue", "sha256:def")
            .record_interval("2026-02-01", "2026-02-02");
        prod_store.save_intervals(&prod_intervals_to_write).unwrap();

        let dev_reloaded = dev_store.load_intervals().unwrap();
        assert_eq!(
            dev_reloaded
                .get("daily_revenue")
                .unwrap()
                .covered_intervals
                .len(),
            1
        );
        assert_eq!(
            dev_reloaded.get("daily_revenue").unwrap().covered_intervals[0]
                .start
                .to_string(),
            "2026-01-01"
        );

        // Disjoint on disk too.
        assert!(dir
            .path()
            .join(".smelt/targets/dev/intervals.json")
            .exists());
        assert!(dir
            .path()
            .join(".smelt/targets/prod/intervals.json")
            .exists());
    }

    /// `docs/specs/run_state.md` §"`meta.json` and layout versioning": the
    /// first locked open under a version-aware binary migrates a legacy
    /// root-level layout into `targets/<target>/` for the
    /// target of the run doing the migration. A legacy root-level
    /// `reconciliation.json` — from a pre-residency `.smelt/` — is left in
    /// place: the reconciliation ledger is engine-resident now, so that
    /// name is not a recognised legacy-layout artifact.
    #[test]
    fn legacy_root_state_migrates_to_first_run_target() {
        let dir = TempDir::new().unwrap();
        let smelt_dir = dir.path().join(".smelt");
        std::fs::create_dir_all(smelt_dir.join("runs")).unwrap();
        std::fs::create_dir_all(smelt_dir.join("schemas")).unwrap();
        std::fs::write(smelt_dir.join("runs").join("run1.json"), "{}").unwrap();
        std::fs::write(smelt_dir.join("intervals.json"), "{}").unwrap();
        std::fs::write(smelt_dir.join("reconciliation.json"), "{}").unwrap();
        std::fs::write(smelt_dir.join("landed_deltas.json"), "{}").unwrap();
        std::fs::write(smelt_dir.join("schemas").join("daily_revenue.json"), "{}").unwrap();

        let store = FileStore::new(dir.path(), "prod");
        let _guard = store.lock().unwrap();

        // Every legacy artifact moved under targets/prod/, none left at root.
        let target_dir = smelt_dir.join("targets").join("prod");
        assert!(target_dir.join("runs").join("run1.json").exists());
        assert!(target_dir.join("intervals.json").exists());
        assert!(target_dir.join("landed_deltas.json").exists());
        assert!(target_dir
            .join("schemas")
            .join("daily_revenue.json")
            .exists());
        assert!(!smelt_dir.join("runs").exists());
        assert!(!smelt_dir.join("intervals.json").exists());
        assert!(!smelt_dir.join("landed_deltas.json").exists());
        assert!(!smelt_dir.join("schemas").exists());

        // `reconciliation.json` is not migrated — left in place at the
        // legacy root location, inert.
        assert!(smelt_dir.join("reconciliation.json").exists());

        drop(_guard);

        // Idempotent: locking again (meta.json now exists) is a no-op —
        // nothing left to migrate, nothing errors.
        let _guard2 = store.lock().unwrap();
        assert!(target_dir.join("intervals.json").exists());
    }

    // --- Phase 7: --resume — manifest persists every outcome, including failures ---

    /// `docs/specs/run_state.md` §"Run manifest": every model smelt
    /// attempted or considered in a run has an entry keyed by outcome, and
    /// `--resume` (`docs/plans/20260719-prod-w2-operability.md` Phase 7)
    /// depends on a *failed* run's manifest actually reaching disk — not
    /// just a successful one. `FileStore::save_run` itself has no notion of
    /// "did the run succeed"; it must faithfully persist a manifest with
    /// `completed_at: None` and a mix of `success`/`failed`/`skipped`
    /// entries exactly as given, and every entry's `definition_hash` must
    /// round-trip too, since that is what `--resume` compares to detect an
    /// edited model.
    #[test]
    fn failed_run_manifest_persists_all_outcomes() {
        let dir = TempDir::new().unwrap();
        let store = FileStore::new(dir.path(), "dev");

        let mut models = HashMap::new();
        models.insert(
            "upstream".to_string(),
            ModelRunRecord {
                strategy: "full_refresh".to_string(),
                time_range: None,
                partitions_updated: vec![],
                row_count: 10,
                duration_ms: 5,
                batch_safety: None,
                outcome: crate::RunOutcomeKind::Success,
                definition_hash: "sha256:aaa".to_string(),
                error: None,
                retry_count: 0,
                probes: Vec::new(),
                subsumed: None,
                deferred_cells: Vec::new(),
            },
        );
        models.insert(
            "middle".to_string(),
            ModelRunRecord {
                strategy: "full_refresh".to_string(),
                time_range: None,
                partitions_updated: vec![],
                row_count: 0,
                duration_ms: 0,
                batch_safety: None,
                outcome: crate::RunOutcomeKind::Failed,
                definition_hash: "sha256:bbb".to_string(),
                error: None,
                retry_count: 0,
                probes: Vec::new(),
                subsumed: None,
                deferred_cells: Vec::new(),
            },
        );
        models.insert(
            "downstream".to_string(),
            ModelRunRecord {
                strategy: "skipped".to_string(),
                time_range: None,
                partitions_updated: vec![],
                row_count: 0,
                duration_ms: 0,
                batch_safety: Some("skipped".to_string()),
                outcome: crate::RunOutcomeKind::Skipped,
                definition_hash: "sha256:ccc".to_string(),
                error: None,
                retry_count: 0,
                probes: Vec::new(),
                subsumed: None,
                deferred_cells: Vec::new(),
            },
        );

        // A failed run never sets completed_at — it denotes an incomplete
        // run, which is exactly what `--resume` looks for.
        let manifest = RunManifest {
            run_id: "20260720-100000-fa17ed".to_string(),
            started_at: Utc::now(),
            completed_at: None,
            models,
        };

        store.save_run(&manifest).unwrap();

        let loaded = store
            .load_run(&manifest.run_id)
            .unwrap()
            .expect("failed run's manifest must still be on disk");
        assert!(loaded.completed_at.is_none());
        assert_eq!(loaded.models.len(), 3);
        assert_eq!(
            loaded.models["upstream"].outcome,
            crate::RunOutcomeKind::Success
        );
        assert_eq!(loaded.models["upstream"].definition_hash, "sha256:aaa");
        assert_eq!(
            loaded.models["middle"].outcome,
            crate::RunOutcomeKind::Failed
        );
        assert_eq!(
            loaded.models["downstream"].outcome,
            crate::RunOutcomeKind::Skipped
        );

        // And it shows up via load_runs (the whole-history read used to
        // find the "latest incomplete run" for --resume).
        let all_runs = store.load_runs(None).unwrap();
        assert_eq!(all_runs.len(), 1);
        assert!(all_runs[0].completed_at.is_none());
    }

    // --- Phase 8: state.mode posture gating ---

    #[test]
    fn written_artifacts_match_the_posture_table() {
        use StateArtifact::*;
        assert_eq!(state_artifacts_written(StateMode::Stateless), &[] as &[_]);
        assert_eq!(
            state_artifacts_written(StateMode::Intervals),
            &[
                RunManifest,
                RunReport,
                Intervals,
                LandedDeltas,
                SourcePostures,
                SourceMutations,
                MigrationApprovals,
                FrozenBandBaselines,
                SchemaSnapshot,
            ]
        );
        assert_eq!(
            state_artifacts_written(StateMode::Environments),
            &[
                RunManifest,
                RunReport,
                Intervals,
                LandedDeltas,
                SourcePostures,
                SourceMutations,
                MigrationApprovals,
                FrozenBandBaselines,
                SchemaSnapshot,
                SnapshotStore,
            ]
        );
        // environments is a strict superset of intervals.
        for artifact in state_artifacts_written(StateMode::Intervals) {
            assert!(state_artifacts_written(StateMode::Environments).contains(artifact));
        }
    }

    #[test]
    fn stateless_store_writes_nothing() {
        let dir = TempDir::new().unwrap();
        let store = FileStore::with_state_mode(dir.path(), "dev", StateMode::Stateless);

        store.init().unwrap();
        let _guard = store.lock().unwrap();
        store.save_run(&test_manifest()).unwrap();
        store
            .save_report(&crate::RunReport {
                run_id: "r1".to_string(),
                started_at: Utc::now(),
                completed_at: Some(Utc::now()),
                duration_ms: 0,
                outcome_counts: crate::OutcomeCounts::default(),
                failures: Vec::new(),
            })
            .unwrap();
        store.save_intervals(&IntervalStore::default()).unwrap();
        store
            .save_landed_deltas(&crate::landed_deltas::LandedDeltaStore::default())
            .unwrap();
        store
            .save_source_postures(&SourcePostureStore::default())
            .unwrap();
        store
            .save_source_mutations(&crate::source_mutations::SourceMutationStore::default())
            .unwrap();
        store
            .save_migration_approvals(
                &crate::migration_approvals::MigrationApprovalStore::default(),
            )
            .unwrap();
        store
            .save_frozen_band_baselines(&FrozenBandBaselineStore::default())
            .unwrap();
        store
            .save_schema(&DeployedSchema {
                model: "m".to_string(),
                version: 1,
                deployed_at: Utc::now(),
                model_hash: "sha256:abc".to_string(),
                model_sql: None,
                partition_column: None,
                columns: vec![],
            })
            .unwrap();
        drop(_guard);

        assert!(
            !dir.path().join(".smelt").exists(),
            "stateless posture must leave no .smelt/ entry"
        );
    }

    #[test]
    fn intervals_store_denies_snapshot_store() {
        let dir = TempDir::new().unwrap();
        let intervals_store = FileStore::with_state_mode(dir.path(), "dev", StateMode::Intervals);

        let mut snap = SnapshotStore::default();
        snap.upsert(SnapshotEntry {
            model: "orders".to_string(),
            environment: "prod".to_string(),
            physical_table: "orders__prod".to_string(),
            source_sql: "SELECT 1".to_string(),
            fingerprint_hex: None,
        });
        intervals_store.save_snapshot_store(&snap).unwrap();
        assert!(
            !dir.path()
                .join(".smelt/targets/dev/snapshots.json")
                .exists(),
            "intervals posture must not write the snapshot store"
        );

        let env_dir = TempDir::new().unwrap();
        let env_store = FileStore::with_state_mode(env_dir.path(), "dev", StateMode::Environments);
        env_store.save_snapshot_store(&snap).unwrap();
        assert!(
            env_dir
                .path()
                .join(".smelt/targets/dev/snapshots.json")
                .exists(),
            "environments posture must write the snapshot store"
        );
    }

    #[test]
    fn stateless_loads_return_defaults_over_stale_files() {
        let dir = TempDir::new().unwrap();

        // A prior, higher-posture run left real state behind.
        let env_store = FileStore::with_state_mode(dir.path(), "dev", StateMode::Environments);
        env_store
            .save_intervals(&{
                let mut intervals = IntervalStore::default();
                intervals
                    .get_or_create("m", "sha256:abc")
                    .record_interval("2026-01-01", "2026-01-02");
                intervals
            })
            .unwrap();
        assert!(dir
            .path()
            .join(".smelt/targets/dev/intervals.json")
            .exists());

        // A stateless store over the same project dir must not read it back.
        let stateless_store = FileStore::with_state_mode(dir.path(), "dev", StateMode::Stateless);
        let loaded = stateless_store.load_intervals().unwrap();
        assert!(
            loaded.models.is_empty(),
            "stateless posture must not read back a stale higher-posture file"
        );
    }
}
