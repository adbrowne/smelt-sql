use criterion::{criterion_group, criterion_main, Criterion};
use smelt_bench::model_gen::{generate_workspace, GraphSpec};

fn bench_parse_simple(c: &mut Criterion) {
    let sql =
        "SELECT user_id, event_time, amount FROM smelt.models.events WHERE status = 'active'\n";

    c.bench_function("parse_simple_sql", |b| {
        b.iter(|| {
            let _ = smelt_parser::parse(sql);
        })
    });
}

fn bench_parse_complex(c: &mut Criterion) {
    let sql = r#"WITH filtered AS (
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

    c.bench_function("parse_complex_sql", |b| {
        b.iter(|| {
            let _ = smelt_parser::parse(sql);
        })
    });
}

fn bench_parse_batch(c: &mut Criterion) {
    let spec = GraphSpec::default();
    let workspace = generate_workspace(&spec).expect("Failed to generate workspace");

    // Pre-strip frontmatter
    let stripped: Vec<String> = workspace
        .sql_contents
        .iter()
        .map(|(_, content)| smelt_parser::strip_frontmatter(content))
        .collect();

    let total_bytes: usize = stripped.iter().map(|s| s.len()).sum();

    c.bench_function(
        &format!(
            "parse_batch_{}_models_{}KB",
            stripped.len(),
            total_bytes / 1024
        ),
        |b| {
            b.iter(|| {
                for sql in &stripped {
                    let _ = smelt_parser::parse(sql);
                }
            })
        },
    );
}

criterion_group!(
    benches,
    bench_parse_simple,
    bench_parse_complex,
    bench_parse_batch
);
criterion_main!(benches);
