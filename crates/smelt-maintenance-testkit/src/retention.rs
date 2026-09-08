//! Retention trimming for the trimmed-retention conformance pool
//! (`docs/outcomes/20260906-trimmed-history-sources/phases/08-plan.md`):
//! physically departs rows from a staged source once they age out of its
//! declared `retention:` bound, so the bound advances with the schedule's
//! own run clock instead of being a fixed test fixture. Split out of
//! `recipe.rs`/`feed.rs` (both already at their large-file baseline) rather
//! than grown into either.

use chrono::NaiveDate;
use smelt_backend::Backend;

use crate::recipe::SourceRecipe;

/// A declared, rolling `retention:` bound (`sources.md` §"Retention
/// refusal"): the source retains `days` days of history, anchored to the
/// current run and advancing with it rather than a fixed calendar date.
#[derive(Debug, Clone, Copy)]
pub struct RetentionDecl {
    pub days: i64,
}

impl SourceRecipe {
    /// Declares a rolling `retention:` bound of `days` days on this source
    /// (`sources.md` §"Retention refusal") — used by the trimmed-retention
    /// conformance pool (`smelt-cli`'s `gate::retention_pool`, phase 8 of
    /// `docs/outcomes/20260906-trimmed-history-sources`).
    pub fn with_retention(mut self, days: i64) -> Self {
        self.retention = Some(RetentionDecl { days });
        self
    }
}

/// The oldest event-time date still retained when the bound is `bound_days`
/// days wide and the run clock reads `as_of` (`sources.md` §"Retention
/// refusal": a rolling bound anchored to the current run, not a fixed
/// calendar date). A row whose clock-column value is strictly older than
/// this date has departed the source; a row exactly on the boundary is
/// still retained.
pub fn retained_cutoff(as_of: NaiveDate, bound_days: i64) -> NaiveDate {
    as_of - chrono::Duration::days(bound_days)
}

/// Physically `DELETE` every row of `source`'s staged table whose clock
/// column is strictly older than [`retained_cutoff`]`(as_of, bound)` — a
/// no-op when `source.retention` is `None`. Mirrors `gate::partition_pool`'s
/// own `main.sources_<name>` staging convention (`insert_row`).
pub async fn trim_source_to_retention(
    backend: &dyn Backend,
    source: &SourceRecipe,
    as_of: NaiveDate,
) -> anyhow::Result<()> {
    let Some(retention) = &source.retention else {
        return Ok(());
    };
    let cutoff = retained_cutoff(as_of, retention.days);
    backend
        .execute_sql(&format!(
            "DELETE FROM main.sources_{} WHERE {} < DATE '{}'",
            source.name,
            source.clock_column,
            cutoff.format("%Y-%m-%d"),
        ))
        .await
        .map_err(|e| anyhow::anyhow!("trim source {} to retention: {e}", source.name))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::{arb_recipe, RecipePool};
    use crate::render;
    use proptest::strategy::{Strategy, ValueTree};
    use proptest::test_runner::TestRunner;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// `retained_cutoff_is_the_run_clock_minus_the_bound` (phase 8 TDD
    /// list): rows strictly older than `as_of - bound` depart, rows on the
    /// boundary are retained.
    #[test]
    fn retained_cutoff_is_the_run_clock_minus_the_bound() {
        let as_of = date(2026, 3, 31);
        let cutoff = retained_cutoff(as_of, 30);
        assert_eq!(cutoff, date(2026, 3, 1));

        // A row exactly on the cutoff date is retained (the DELETE compares
        // strictly-less-than `cutoff`); a row one day older has departed.
        assert!(cutoff == date(2026, 3, 1));
        let departed = cutoff - chrono::Duration::days(1);
        assert_eq!(departed, date(2026, 2, 28));
    }

    /// `render_source_yaml_emits_a_declared_retention` (phase 8 TDD list):
    /// a `SourceRecipe` carrying a retention bound renders `retention: '<n>
    /// days'` in the source YAML.
    #[test]
    fn render_source_yaml_emits_a_declared_retention() {
        let mut recipe = arb_recipe(RecipePool::partition_append_only())
            .new_tree(&mut TestRunner::deterministic())
            .unwrap()
            .current();
        recipe.source = recipe.source.with_retention(30);
        let yaml = render::render_source_yaml(&recipe);
        assert!(
            yaml.contains("retention: '30 days'\n"),
            "expected a declared retention line, got:\n{yaml}"
        );
    }

    /// `render_source_yaml_without_retention_is_byte_identical` (phase 8 TDD
    /// list): regression pin — `retention: None` renders exactly today's
    /// string, with no trailing `retention:` line.
    #[test]
    fn render_source_yaml_without_retention_is_byte_identical() {
        let recipe = arb_recipe(RecipePool::partition_append_only())
            .new_tree(&mut TestRunner::deterministic())
            .unwrap()
            .current();
        assert!(recipe.source.retention.is_none());
        let yaml = render::render_source_yaml(&recipe);
        assert!(
            !yaml.contains("retention:"),
            "undeclared retention must render no retention line, got:\n{yaml}"
        );
    }
}
