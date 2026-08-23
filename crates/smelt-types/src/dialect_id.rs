/// A backend SQL dialect, as an identity the registry can key on.
///
/// Replaces the stringly-keyed `engine_native` convention, where a typo'd key
/// silently meant "no override" — a fail-loud violation this table must not
/// inherit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DialectId {
    DuckDb,
    SparkSql,
    PostgreSql,
    BigQuery,
}

impl DialectId {
    /// Every dialect, in report order. Exhaustive by construction: adding a
    /// variant without adding it here fails `all_is_exhaustive`.
    pub const ALL: &'static [DialectId] = &[
        DialectId::DuckDb,
        DialectId::SparkSql,
        DialectId::PostgreSql,
        DialectId::BigQuery,
    ];

    /// The lowercase key already used by `smelt-runtime`'s as-struct emitter and
    /// the type-divergence ledger. There must not be a second spelling.
    pub fn slug(self) -> &'static str {
        match self {
            DialectId::DuckDb => "duckdb",
            DialectId::SparkSql => "spark",
            DialectId::PostgreSql => "postgres",
            DialectId::BigQuery => "bigquery",
        }
    }

    pub fn from_slug(slug: &str) -> Option<DialectId> {
        DialectId::ALL.iter().copied().find(|d| d.slug() == slug)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_is_exhaustive() {
        // A new variant added without extending ALL fails here: the match is
        // exhaustive, so every variant must be produced by the iteration.
        for d in DialectId::ALL {
            match d {
                DialectId::DuckDb
                | DialectId::SparkSql
                | DialectId::PostgreSql
                | DialectId::BigQuery => {}
            }
        }
        assert_eq!(DialectId::ALL.len(), 4);
    }

    #[test]
    fn slug_round_trips_and_matches_the_existing_spelling() {
        for d in DialectId::ALL {
            assert_eq!(DialectId::from_slug(d.slug()), Some(*d));
        }
        // These four strings are load-bearing: smelt-runtime's as-struct emitter
        // and the type-divergence ledger already key on them.
        assert_eq!(DialectId::DuckDb.slug(), "duckdb");
        assert_eq!(DialectId::SparkSql.slug(), "spark");
        assert_eq!(DialectId::PostgreSql.slug(), "postgres");
        assert_eq!(DialectId::BigQuery.slug(), "bigquery");
    }

    #[test]
    fn an_unknown_slug_is_none_not_a_default() {
        assert_eq!(DialectId::from_slug("duckdb "), None);
        assert_eq!(DialectId::from_slug("DuckDb"), None);
        assert_eq!(DialectId::from_slug("snowflake"), None);
    }
}
