use crate::model_gen::GeneratedWorkspace;
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Metrics from parser throughput benchmarks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParserMetrics {
    /// Time to parse a single simple model (microseconds).
    pub single_simple_us: f64,
    /// Time to parse a single complex model (microseconds).
    pub single_complex_us: f64,
    /// Time to parse all SQL models in batch (milliseconds).
    pub batch_all_ms: f64,
    /// Total bytes of SQL parsed in batch.
    pub total_bytes: usize,
    /// Throughput: bytes per second.
    pub bytes_per_second: f64,
    /// Number of models parsed in batch.
    pub batch_count: usize,
}

/// A simple SQL model for single-parse timing.
const SIMPLE_SQL: &str =
    "SELECT user_id, event_time, amount FROM smelt.models.events WHERE status = 'active'\n";

/// A complex SQL model for single-parse timing.
const COMPLEX_SQL: &str = r#"WITH filtered AS (
    SELECT user_id, event_time, amount, category
    FROM smelt.models.events
    WHERE status = 'active'
      AND event_time >= '2024-01-01'
),
aggregated AS (
    SELECT
        user_id,
        category,
        COUNT(*) AS event_count,
        SUM(amount) AS total_amount,
        AVG(amount) AS avg_amount,
        MIN(event_time) AS first_event,
        MAX(event_time) AS last_event
    FROM filtered
    GROUP BY user_id, category
    HAVING COUNT(*) > 5
)
SELECT
    a.user_id,
    a.category,
    a.event_count,
    a.total_amount,
    a.avg_amount,
    ROW_NUMBER() OVER (PARTITION BY a.category ORDER BY a.total_amount DESC) AS rank_in_category
FROM aggregated a
INNER JOIN smelt.models.users u ON a.user_id = u.user_id
WHERE u.is_active = true
ORDER BY a.total_amount DESC
LIMIT 1000
"#;

/// Run parser throughput benchmarks.
pub fn run_parser_benchmark(workspace: &GeneratedWorkspace) -> ParserMetrics {
    // Single simple parse
    let simple_start = Instant::now();
    for _ in 0..100 {
        let _ = smelt_parser::parse(SIMPLE_SQL);
    }
    let single_simple_us = simple_start.elapsed().as_secs_f64() * 1_000_000.0 / 100.0;

    // Single complex parse
    let complex_start = Instant::now();
    for _ in 0..100 {
        let _ = smelt_parser::parse(COMPLEX_SQL);
    }
    let single_complex_us = complex_start.elapsed().as_secs_f64() * 1_000_000.0 / 100.0;

    // Batch parse all SQL models
    let total_bytes: usize = workspace.sql_contents.iter().map(|(_, c)| c.len()).sum();
    let batch_count = workspace.sql_contents.len();

    let batch_start = Instant::now();
    for (_, content) in &workspace.sql_contents {
        let stripped = smelt_parser::strip_frontmatter(content);
        let _ = smelt_parser::parse(&stripped);
    }
    let batch_all_ms = batch_start.elapsed().as_secs_f64() * 1000.0;

    let bytes_per_second = if batch_all_ms > 0.0 {
        total_bytes as f64 / (batch_all_ms / 1000.0)
    } else {
        0.0
    };

    ParserMetrics {
        single_simple_us,
        single_complex_us,
        batch_all_ms,
        total_bytes,
        bytes_per_second,
        batch_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_gen::{generate_workspace, GraphSpec};

    #[test]
    fn test_parser_benchmark_small() {
        let spec = GraphSpec::small();
        let workspace = generate_workspace(&spec).unwrap();
        let metrics = run_parser_benchmark(&workspace);

        assert!(metrics.single_simple_us > 0.0);
        assert!(metrics.single_complex_us > 0.0);
        assert!(metrics.batch_count > 0);
        assert!(metrics.bytes_per_second > 0.0);
    }
}
