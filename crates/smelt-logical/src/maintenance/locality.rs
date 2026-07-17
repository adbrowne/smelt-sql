//! Key temporal locality gate
//! (`docs/specs/incremental_models.md` §"Key temporal locality (the
//! time-partitioned output)").
//!
//! A keyed model (`grain: key`) may time-partition its output with a
//! `timeseries:` block; admission requires **key temporal locality** — a
//! guarantee that every stored row a run's deltas can touch lies within a
//! computable slice of the output's time axis. Three routes can establish
//! it (key-embedded, key-determined, recurrence-bounded); a model that
//! satisfies none of them is refused.
//!
//! [`establish_locality`] is the **single entry point** for this decision
//! (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase
//! A1's review checklist: "no second place decides keyed+timeseries
//! admissibility"). This module currently implements no route — every call
//! refuses with [`LocalityRefusal::NoRouteEstablished`] — but the seam is
//! built to survive each route landing behind it unchanged: later phases
//! widen the body of `establish_locality`, not its signature or its
//! callers.
//!
//! Pure module: no I/O, no Salsa. `smelt-db`'s `maintenance_plan` query
//! (`crates/smelt-db/src/queries/maintenance.rs`) is the sole caller.

/// Inputs the locality gate consults. Only the model name is threaded today
/// — every call refuses regardless of the model's actual shape. Later
/// phases add the structural facts each route's admission needs (declared
/// `unique_key`, the partition column's provenance family, a derived or
/// declared key-recurrence bound, …).
#[derive(Debug, Clone)]
pub struct LocalityInputs {
    /// The model name, folded into the refusal message.
    pub model_name: String,
}

/// The established locality slice a `merge_into` target scan may be pruned
/// to (`incremental_models.md` §"Key temporal locality"). Not yet
/// constructed anywhere — no route is implemented — but declared now so
/// [`establish_locality`]'s signature does not change when one is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalitySlice;

/// Why key temporal locality could not be established for a model's
/// `timeseries:` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalityRefusal {
    /// None of the three routes (key-embedded, key-determined,
    /// recurrence-bounded) applies. `nearest_missing_fact` names the
    /// single fact closest to being satisfied, to focus the fix — the
    /// spec's diagnostic contract: "The message names the three routes and
    /// the nearest missing fact" (`incremental_models.md` §Diagnostics,
    /// `KeyedForbidsTimeseries`).
    NoRouteEstablished { nearest_missing_fact: String },
}

impl LocalityRefusal {
    /// Render the refusal as the `KeyedForbidsTimeseries` diagnostic
    /// message: names all three routes and the nearest missing fact.
    pub fn message(&self, model_name: &str) -> String {
        match self {
            LocalityRefusal::NoRouteEstablished {
                nearest_missing_fact,
            } => format!(
                "KeyedForbidsTimeseries: model '{model_name}' declares a `timeseries:` block \
                 but key temporal locality could not be established — no route applies. \
                 The three routes: \
                 (1) key-embedded — `partition_column` is itself a `unique_key` column; \
                 (2) key-determined — the partition projection is a per-key constant, proven by \
                 once-write provenance (a key-derived expression or a declared functional \
                 dependency over a provably non-null column); \
                 (3) recurrence-bounded — a key-recurrence bound `r` holds (statically derived, \
                 or declared on the driving source via `key_recurrence`), so every pair of rows \
                 sharing a key lies within `r` of each other on the event-time axis. \
                 Nearest missing fact: {nearest_missing_fact}."
            ),
        }
    }
}

impl std::fmt::Display for LocalityRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message("<model>"))
    }
}

/// Establish key temporal locality for a keyed model's `timeseries:`
/// block.
///
/// This phase implements no route: every call refuses with
/// [`LocalityRefusal::NoRouteEstablished`]. Later phases (route 1: A2,
/// route 2: A3, route 3: A4) replace this body with the routes' real
/// structural preconditions and admission logic — this function is the
/// single seam every route lands in
/// (`docs/specs/incremental_models.md` §"Key temporal locality").
pub fn establish_locality(_inputs: &LocalityInputs) -> Result<LocalitySlice, LocalityRefusal> {
    Err(LocalityRefusal::NoRouteEstablished {
        nearest_missing_fact: "no route's structural preconditions are implemented yet \
             (key-embedded, key-determined, and recurrence-bounded admission all land in \
             docs/plans/20260715-composed-axes-conditional-maintenance.md, phases A2-A4)"
            .to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs() -> LocalityInputs {
        LocalityInputs {
            model_name: "device_daily".to_string(),
        }
    }

    /// With no route implemented, every call refuses.
    #[test]
    fn establish_locality_always_refuses() {
        let result = establish_locality(&inputs());
        assert!(
            matches!(result, Err(LocalityRefusal::NoRouteEstablished { .. })),
            "expected NoRouteEstablished, got: {:?}",
            result
        );
    }

    /// The rendered message names all three routes and the nearest missing
    /// fact — the spec's diagnostic contract for `KeyedForbidsTimeseries`.
    #[test]
    fn refusal_message_names_all_three_routes_and_nearest_missing_fact() {
        let err = establish_locality(&inputs()).unwrap_err();
        let message = err.message("device_daily");

        assert!(
            message.contains("KeyedForbidsTimeseries"),
            "message must carry the diagnostic code: {message}"
        );
        assert!(
            message.to_lowercase().contains("key-embedded"),
            "message must name route 1 (key-embedded): {message}"
        );
        assert!(
            message.to_lowercase().contains("key-determined"),
            "message must name route 2 (key-determined): {message}"
        );
        assert!(
            message.to_lowercase().contains("recurrence-bounded"),
            "message must name route 3 (recurrence-bounded): {message}"
        );
        assert!(
            message.contains("Nearest missing fact"),
            "message must name the nearest missing fact: {message}"
        );
    }
}
