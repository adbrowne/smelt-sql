//! Serialises the clear/exec/collect critical section of
//! [`super::run_python_model_file`].
//!
//! The embedded interpreter is process-global, and so is the registry the
//! `@model` decorator writes to (`smelt.core._registered_models`). The GIL
//! alone does **not** protect that section: CPython drops the GIL on every
//! I/O call a model makes, so two concurrent `run_python_model_file` calls
//! interleave — A clears, A execs, B clears (dropping A's registration and
//! adding its own), then A iterates the registry and runs *B's* model. The
//! observable symptoms are a model file returning another file's models,
//! and an error attributed to one file carrying another file's traceback
//! (issue #189).

use pyo3::prelude::*;
use std::sync::{Condvar, Mutex};

/// Acquire this **with the GIL released** (see [`lock_registry`]): a thread
/// that held the GIL while blocking here would deadlock against the lock
/// holder waiting to re-acquire the GIL.
static REGISTRY_HELD: Mutex<bool> = Mutex::new(false);
static REGISTRY_CV: Condvar = Condvar::new();

/// RAII release for the registry lock acquired by [`lock_registry`].
pub(super) struct RegistryGuard;

impl Drop for RegistryGuard {
    fn drop(&mut self) {
        let mut held = REGISTRY_HELD.lock().unwrap_or_else(|e| e.into_inner());
        *held = false;
        REGISTRY_CV.notify_one();
    }
}

/// Acquire [`REGISTRY_HELD`] with the GIL released.
///
/// A plain `MutexGuard` cannot cross `Python::detach` (it is not `Send`), so the
/// wait is expressed as a condvar gate whose guard stays inside the closure and
/// the ownership flag is handed back through the `RegistryGuard`.
pub(super) fn lock_registry(py: Python<'_>) -> RegistryGuard {
    py.detach(|| {
        let mut held = REGISTRY_HELD.lock().unwrap_or_else(|e| e.into_inner());
        while *held {
            held = REGISTRY_CV.wait(held).unwrap_or_else(|e| e.into_inner());
        }
        *held = true;
    });
    RegistryGuard
}
