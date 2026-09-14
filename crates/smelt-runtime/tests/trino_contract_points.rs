//! Phase 6 (`docs/outcomes/20260913-trino-ledger/phases/06-plan.md`): the
//! `contract.frozen_horizon` maintenance-dialect resolution gap at
//! `execute/project/mod.rs`'s live-dispatch site. `smelt_backend::
//! maintenance_dialect(backend.dialect())?` used to resolve unconditionally
//! inside the `if let Some(end_date)` arm, before `frozen_horizon_probes`
//! was ever asked whether the model declares anything — so a Trino run of
//! ANY clocked model (declared or not) would have hard-errored with
//! `UnsupportedMaintenanceDialect`, since Trino currently has no
//! `MaintenanceDialect` mapping at all (`20260913-trino-incremental` owns
//! adding one). The fix, `crate::contract_probes::
//! resolve_frozen_horizon_dialect`, resolves the dialect only when
//! `contract.frozen_horizon` is actually declared, and where the target has
//! no maintenance dialect, skips the verification probe with a
//! `tracing::warn!` instead of propagating the error — the declaration
//! itself stays valid (`docs/specs/state.md` §"Declarations stay
//! fail-loud"), only its live verification is unavailable.
//!
//! Tested directly against the extracted pure/logging boundary rather than
//! through a full `execute_project` run: Trino has no `MaintenanceDialect`
//! mapping yet at all, so ANY live incremental batch write (not just a
//! frozen-horizon-declaring one) still hard-errors downstream today at the
//! DELETE+INSERT emission site — that is `20260913-trino-incremental`'s
//! subject, not this phase's. `resolve_frozen_horizon_dialect` is the
//! narrow, already-testable seam this phase's fix actually lives behind.

use std::cell::RefCell;
use std::sync::{Once, OnceLock};

use smelt_backend::SqlDialect;
use smelt_core::config::{ContractConfig, DataLatency};
use smelt_core::ModelMetadata;
use smelt_runtime::contract_probes::resolve_frozen_horizon_dialect;
use tracing_subscriber::layer::SubscriberExt;

fn metadata_without_frozen_horizon() -> ModelMetadata {
    ModelMetadata::default()
}

fn metadata_with_frozen_horizon() -> ModelMetadata {
    ModelMetadata {
        contract: Some(ContractConfig {
            frozen_horizon: DataLatency::parse("90 days"),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// RED test for the 3554 gap: a clocked model with NO `contract.
/// frozen_horizon` must never even ask `smelt_backend::maintenance_dialect`
/// for a Trino mapping — it returns `None` (skip) immediately, and (unlike
/// the pre-fix call site) never has a chance to propagate
/// `UnsupportedMaintenanceDialect`.
#[test]
fn undeclared_frozen_horizon_never_resolves_a_dialect_on_trino() {
    let metadata = metadata_without_frozen_horizon();
    let resolved =
        resolve_frozen_horizon_dialect("order_totals", SqlDialect::Trino, Some(&metadata));
    assert_eq!(
        resolved, None,
        "an undeclared contract.frozen_horizon must never pay for a dialect that does not \
         exist on Trino"
    );
}

/// The same undeclared case on DuckDB (which DOES have a maintenance
/// dialect) also returns `None` — the gate is purely "was it declared?",
/// never a dialect-availability check for the undeclared case.
#[test]
fn undeclared_frozen_horizon_never_resolves_a_dialect_on_duckdb() {
    let metadata = metadata_without_frozen_horizon();
    let resolved =
        resolve_frozen_horizon_dialect("order_totals", SqlDialect::DuckDB, Some(&metadata));
    assert_eq!(resolved, None);
}

/// With the declaration present on a dialect that DOES realise a
/// maintenance dialect (DuckDB), the probe's dialect resolves normally —
/// non-vacuity for the skip below.
#[test]
fn declared_frozen_horizon_resolves_on_duckdb() {
    let metadata = metadata_with_frozen_horizon();
    let resolved =
        resolve_frozen_horizon_dialect("order_totals", SqlDialect::DuckDB, Some(&metadata));
    assert_eq!(resolved, Some(smelt_backend::MaintenanceDialect::DuckDb));
}

// Per-thread WARN buffer, mirroring
// `crates/smelt-runtime/tests/fingerprint_sidecar.rs`'s `capture_warnings`
// harness — see that file's doc comments for why the subscriber must be
// installed globally rather than thread-scoped.
thread_local! {
    static CAPTURED_WARNINGS: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

struct CapturingLayer;

fn install_capturing_subscriber() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let subscriber = tracing_subscriber::registry().with(CapturingLayer);
        tracing::subscriber::set_global_default(subscriber)
            .expect("install the WARN-capturing global subscriber");
    });
}

fn capture_warnings<T>(f: impl FnOnce() -> T) -> (T, Vec<String>) {
    install_capturing_subscriber();
    CAPTURED_WARNINGS.with(|c| *c.borrow_mut() = Some(Vec::new()));
    let output = f();
    let captured = CAPTURED_WARNINGS
        .with(|c| c.borrow_mut().take())
        .expect("WARN capture buffer must still be installed on this thread");
    (output, captured)
}

impl<S> tracing_subscriber::Layer<S> for CapturingLayer
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        if *event.metadata().level() != tracing::Level::WARN {
            return;
        }
        struct MessageVisitor<'a>(&'a mut String);
        impl tracing::field::Visit for MessageVisitor<'_> {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                use std::fmt::Write;
                let _ = write!(self.0, " {}={:?}", field.name(), value);
            }
        }
        let mut message = String::new();
        event.record(&mut MessageVisitor(&mut message));
        CAPTURED_WARNINGS.with(|c| {
            if let Some(buffer) = c.borrow_mut().as_mut() {
                buffer.push(message);
            }
        });
    }
}

/// A test-wide lock serialising the two tests below: the WARN-capturing
/// subscriber is process-global (see `install_capturing_subscriber`'s doc
/// comment) and `CAPTURED_WARNINGS` is thread-local, so two tests running
/// concurrently on the SAME thread (the default single-threaded `cargo
/// test` runner reuses worker threads across tests) could otherwise
/// interleave. `cargo test` runs test functions in separate threads by
/// default, but pin this explicitly rather than relying on that.
fn capture_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

/// With the declaration present on Trino (no maintenance dialect), the
/// probe is skipped — `None`, not an error — and exactly one WARN names the
/// model, the declaration and the dialect.
#[test]
fn declared_frozen_horizon_on_trino_skips_with_a_warning() {
    let _guard = capture_lock().lock().unwrap();
    let metadata = metadata_with_frozen_horizon();
    let (resolved, warnings) = capture_warnings(|| {
        resolve_frozen_horizon_dialect("order_totals", SqlDialect::Trino, Some(&metadata))
    });

    assert_eq!(
        resolved, None,
        "a declared contract.frozen_horizon must degrade to a skipped probe on Trino, not an \
         error"
    );
    assert!(
        warnings.iter().any(|w| {
            w.contains("order_totals") && w.contains("frozen_horizon") && w.contains("Trino")
        }),
        "expected a WARN naming the model, the declaration and the dialect, got: {warnings:?}"
    );
}
