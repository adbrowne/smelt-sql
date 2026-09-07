//! The compile path refuses a construct the registry declares unsupported on
//! the target's dialect, rather than emitting SQL the engine rejects.
//!
//! `smelt-dialect`'s `unsupported_emission` suite proves the pure check; this
//! suite proves it is actually wired into every `SqlCompiler` print, so a
//! model never reaches a warehouse carrying a construct that warehouse
//! cannot express.

mod doc_sync;
mod fixtures;
mod refusals;
mod seam;
mod structural;
