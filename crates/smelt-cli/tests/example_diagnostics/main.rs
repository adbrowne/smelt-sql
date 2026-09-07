//! Verify that non-broken example workspaces produce zero LSP diagnostics.
//!
//! This test ensures that `file_diagnostics()` and `check_type_diagnostics()`
//! report no warnings or errors for any model in the example workspaces.
//! Regressions introduced by parser, type-inference, or example changes are
//! caught here.
#![allow(dead_code, unused_imports)]

mod support;
mod support_ext;

mod alias_arity;
mod emission_body;
mod event_time_and_grain;
mod keyed_frontmatter;
mod meta_columns_and_broken_workspace;
mod meta_config;
mod meta_hofs;
mod meta_lists;
mod meta_workspace;
mod per_cohort_union;
mod smoke_and_migration;
mod timeseries_incremental;
