//! Live observed-delta consumption (`docs/outcomes/
//! 20260816-scheduler-delta-signatures/outcome.md` phase 6): the backend
//! read `--since-upstream` was always missing — `propagation::
//! observed_delta_keys_to_read` (pure) says WHICH `(model, window)` keys
//! matter; this module reads them off the warehouse and assembles the
//! `ObservedDeltaLookup` `propagation::plan_since_upstream_with_observed_deltas`
//! consumes.

use anyhow::Result;
use smelt_backend::Backend;

use crate::maintenance_driver::read_observed_delta;
use crate::propagation::{ObservedDeltaKey, ObservedDeltaLookup};

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
