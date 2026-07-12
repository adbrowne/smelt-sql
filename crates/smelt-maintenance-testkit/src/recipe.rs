//! `ModelRecipe` — a typed proptest value for the partition-grain
//! append-only pool (`docs/plans/20260712-generative-maintenance-conformance.md`
//! Phase 1; `docs/research/20260711-generative-maintenance-conformance.md` §4
//! "`ModelRecipe` — generating models as typed data").
//!
//! A recipe is data, not SQL text: [`ModelRecipe`] pairs one [`SourceRecipe`]
//! (an append-only, clocked `events(d, id, val)`-shaped source) with one
//! [`BodyConstruct`] drawn from the coverage-matrix construct axis
//! (pass-through · filter · additive aggregate · idempotent aggregate ·
//! decomposed aggregate (`AVG`) · holistic aggregate) and a [`GrainDecl`]
//! describing the `grain: partition` output shape. [`render`] turns a recipe
//! into SQL/YAML text; this module owns only the typed value and its
//! generator.
//!
//! Valid-by-construction (design §4): every field is a typed enum/struct, not
//! a string the generator could mangle into unparseable SQL, so a generated
//! recipe is expected to render, parse, and type-check cleanly — a recipe
//! that trips a non-maintenance diagnostic is a generator bug (asserted by
//! `render::rendered_recipe_stages_cleanly`), never silently discarded.

use proptest::prelude::*;

/// Bound for generated integer payload literals (design §5 "Numeric payload
/// discipline"): integer-valued payloads with magnitude at most this bound
/// keep additive folds bit-exact well under 2^53, so incremental-vs-full
/// `EXCEPT ALL` comparisons stay exact regardless of fold order. Doubles /
/// variance-class combiners are excluded from the v1 pool for the same
/// reason (design §5).
pub const PAYLOAD_BOUND: i64 = 1_000;

/// A `Strategy` producing integer-valued payload literals in
/// `[-PAYLOAD_BOUND, PAYLOAD_BOUND]` (design §5's numeric discipline). Shared
/// by every pool construct that needs a literal (today: [`BodyConstruct::Filter`]'s
/// threshold); future phases' row-data generation reuses the same bound.
pub fn arb_payload_value() -> impl Strategy<Value = i64> {
    -PAYLOAD_BOUND..=PAYLOAD_BOUND
}

/// The shape of a clocked source's declared `batched.unique_key`
/// (`reachability_sample_inhabits_every_pool_construct` requires both shapes
/// be reachable). Only [`BodyConstruct::PassThrough`] and
/// [`BodyConstruct::Filter`] project a row per source row, so only they use
/// this to pick the declared key; the aggregate constructs always key on the
/// partition column alone (one row per partition, by construction of
/// `GROUP BY`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyShape {
    /// `unique_key: [id]` — the source's own row key alone.
    Single,
    /// `unique_key: [d, id]` — partition column plus row key.
    Composite,
}

fn arb_key_shape() -> impl Strategy<Value = KeyShape> {
    prop_oneof![Just(KeyShape::Single), Just(KeyShape::Composite)]
}

/// The construct drawn from the coverage-matrix construct axis
/// (`docs/research/20260711-generative-maintenance-conformance.md` §4). Phase
/// 1's pool covers the six named in the plan's Phase 1 goal; adversarial
/// (refusal-branch) variants are Phase 2 scope, added to this enum then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyConstruct {
    /// `SELECT d, id, val FROM smelt.sources.<src>` — row-for-row passthrough.
    PassThrough,
    /// Passthrough with a `WHERE val > <threshold>` predicate. `threshold` is
    /// drawn from [`arb_payload_value`] — the payload literal
    /// `payloads_are_integer_valued_and_bounded` checks.
    Filter { threshold: i64 },
    /// `SUM(val) GROUP BY d` — a commutative-group combiner (maintenance
    /// ladder rung 1/3, `maintenance_plan.md` §"The algebraic maintenance
    /// ladder").
    AdditiveAgg,
    /// `MAX(val) GROUP BY d` — an idempotent, non-invertible monoid combiner
    /// (ladder rung 1, not a group).
    IdempotentAgg,
    /// `AVG(val) GROUP BY d` — a decomposed monoid: the user value is a pure
    /// presentation of a richer state (`sum`, `count`) (ladder rung 2).
    DecomposedAgg,
    /// `MEDIAN(val)` + `COUNT(DISTINCT id) GROUP BY d` — holistic aggregates
    /// needing the full per-partition row set (ladder rung 4).
    HolisticAgg,
}

/// Stable, human-readable identifier for a construct — used both for model
/// naming (`recipe_<kind>`) and coverage-matrix cell ids
/// (`recipe_names_its_matrix_cells`).
fn construct_kind_name(construct: BodyConstruct) -> &'static str {
    match construct {
        BodyConstruct::PassThrough => "pass_through",
        BodyConstruct::Filter { .. } => "filter",
        BodyConstruct::AdditiveAgg => "additive_agg",
        BodyConstruct::IdempotentAgg => "idempotent_agg",
        BodyConstruct::DecomposedAgg => "decomposed_agg",
        BodyConstruct::HolisticAgg => "holistic_agg",
    }
}

impl BodyConstruct {
    /// Whether this construct projects one output row per source row
    /// (`PassThrough`/`Filter`) rather than one row per partition (the
    /// aggregate family) — decides whether [`KeyShape`] affects the declared
    /// `unique_key`.
    fn is_row_shaped(self) -> bool {
        matches!(
            self,
            BodyConstruct::PassThrough | BodyConstruct::Filter { .. }
        )
    }

    /// The coverage-matrix cell id(s) this construct inhabits, `construct ×
    /// source-property` (design §4 "Matrix-aware"). Phase 1's pool is
    /// exclusively append-only sources, so every construct maps to exactly
    /// one cell today; a construct crossed with more source properties in a
    /// later phase would return more than one id, which is why this returns
    /// a `Vec` rather than a single id.
    pub fn matrix_cell_ids(self) -> Vec<String> {
        vec![format!("{}×append_only", construct_kind_name(self))]
    }
}

/// The pool of [`BodyConstruct`] kinds [`arb_recipe`] draws from — a
/// unit-only mirror of [`BodyConstruct`] so the pool can be enumerated and
/// sampled without needing a `threshold` value up front (that is filled in by
/// [`arb_recipe`] itself, from [`arb_payload_value`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstructKind {
    PassThrough,
    Filter,
    AdditiveAgg,
    IdempotentAgg,
    DecomposedAgg,
    HolisticAgg,
}

/// A named set of eligible [`ConstructKind`]s [`arb_recipe`] draws from.
/// Phase 1 ships exactly one pool (the partition-grain append-only
/// six-construct set); later phases (adversarial refusal leaves, mutable
/// sources) add pools rather than growing this one, so a cell can pin exactly
/// which constructs it wants reachable.
#[derive(Debug, Clone)]
pub struct RecipePool {
    pub constructs: Vec<ConstructKind>,
}

impl RecipePool {
    /// The Phase 1 pool: pass-through · filter · additive agg · idempotent
    /// agg · decomposed agg (`AVG`) · holistic agg, over the partition-grain
    /// append-only shape (plan Phase 1 goal).
    pub fn partition_append_only() -> Self {
        Self {
            constructs: vec![
                ConstructKind::PassThrough,
                ConstructKind::Filter,
                ConstructKind::AdditiveAgg,
                ConstructKind::IdempotentAgg,
                ConstructKind::DecomposedAgg,
                ConstructKind::HolisticAgg,
            ],
        }
    }
}

/// The one staged source every Phase 1 recipe uses: an append-only, clocked
/// `events(d DATE, id INTEGER, val INTEGER)` source — the same column shape
/// `model_shapes.rs` uses elsewhere in this crate, with `val` narrowed to
/// `INTEGER` per design §5's numeric-payload discipline (a `DOUBLE` payload
/// would make additive folds order-sensitive).
#[derive(Debug, Clone)]
pub struct SourceRecipe {
    pub name: String,
    pub clock_column: String,
    pub key_column: String,
    pub payload_column: String,
    pub key_shape: KeyShape,
}

impl SourceRecipe {
    fn events(key_shape: KeyShape) -> Self {
        Self {
            name: "events".to_string(),
            clock_column: "d".to_string(),
            key_column: "id".to_string(),
            payload_column: "val".to_string(),
            key_shape,
        }
    }

    /// The declared `batched.unique_key` for `construct` over this source:
    /// row-shaped constructs use [`KeyShape`]; aggregate constructs always
    /// key on the partition column alone.
    pub fn unique_key_for(&self, construct: BodyConstruct) -> Vec<String> {
        if construct.is_row_shaped() {
            match self.key_shape {
                KeyShape::Single => vec![self.key_column.clone()],
                KeyShape::Composite => vec![self.clock_column.clone(), self.key_column.clone()],
            }
        } else {
            vec![self.clock_column.clone()]
        }
    }
}

/// The `grain: partition` output declaration: `timeseries:` block +
/// `batched.unique_key`. Phase 1's pool is partition-grain only (plan Phase 1
/// goal); a `grain: key` variant is future scope.
#[derive(Debug, Clone)]
pub struct GrainDecl {
    pub event_time_column: String,
    pub partition_column: String,
    pub granularity: String,
    pub unique_key: Vec<String>,
}

/// A fully-typed model recipe: one source, one body construct, one grain
/// declaration, ready for [`crate::render`] to turn into SQL/YAML text.
#[derive(Debug, Clone)]
pub struct ModelRecipe {
    pub model_name: String,
    pub source: SourceRecipe,
    pub grain: GrainDecl,
    pub construct: BodyConstruct,
}

impl ModelRecipe {
    fn new(construct: BodyConstruct, key_shape: KeyShape) -> Self {
        let source = SourceRecipe::events(key_shape);
        let unique_key = source.unique_key_for(construct);
        let grain = GrainDecl {
            event_time_column: source.clock_column.clone(),
            partition_column: source.clock_column.clone(),
            granularity: "day".to_string(),
            unique_key,
        };
        Self {
            model_name: format!("recipe_{}", construct_kind_name(construct)),
            source,
            grain,
            construct,
        }
    }
}

/// The typed generator: draws a [`ConstructKind`] from `pool`, a
/// [`KeyShape`], and (when the construct is [`ConstructKind::Filter`]) a
/// payload threshold from [`arb_payload_value`], and assembles a
/// [`ModelRecipe`]. Structural shrinking (proptest shrinks the drawn kind /
/// threshold, never the rendered SQL text — design §4 "Structural
/// shrinking") falls out of composing `Strategy`s rather than generating SQL
/// directly.
pub fn arb_recipe(pool: RecipePool) -> impl Strategy<Value = ModelRecipe> {
    (
        proptest::sample::select(pool.constructs),
        arb_key_shape(),
        arb_payload_value(),
    )
        .prop_map(|(kind, key_shape, threshold)| {
            let construct = match kind {
                ConstructKind::PassThrough => BodyConstruct::PassThrough,
                ConstructKind::Filter => BodyConstruct::Filter { threshold },
                ConstructKind::AdditiveAgg => BodyConstruct::AdditiveAgg,
                ConstructKind::IdempotentAgg => BodyConstruct::IdempotentAgg,
                ConstructKind::DecomposedAgg => BodyConstruct::DecomposedAgg,
                ConstructKind::HolisticAgg => BodyConstruct::HolisticAgg,
            };
            ModelRecipe::new(construct, key_shape)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render;
    use proptest::strategy::{Strategy, ValueTree};
    use proptest::test_runner::TestRunner;
    use std::collections::HashSet;

    /// `payloads_are_integer_valued_and_bounded` (plan Phase 1 TDD list):
    /// generated payload literals (today: `Filter`'s threshold) are
    /// integer-valued and bounded by `PAYLOAD_BOUND` (design §5).
    #[test]
    fn payloads_are_integer_valued_and_bounded() {
        let mut runner = TestRunner::deterministic();
        let strat = arb_recipe(RecipePool::partition_append_only());
        let mut saw_filter = false;
        for _ in 0..500 {
            let recipe = strat.new_tree(&mut runner).unwrap().current();
            if let BodyConstruct::Filter { threshold } = recipe.construct {
                saw_filter = true;
                assert!(
                    (-PAYLOAD_BOUND..=PAYLOAD_BOUND).contains(&threshold),
                    "filter threshold {threshold} exceeds the documented bound \
                     ±{PAYLOAD_BOUND} (design §5's numeric-payload discipline)"
                );
            }
        }
        assert!(saw_filter, "sample never generated a Filter recipe");
    }

    /// `reachability_sample_inhabits_every_pool_construct` (plan Phase 1 TDD
    /// list): a deterministic `TestRunner` sample of N=200 recipes inhabits
    /// every `BodyConstruct` variant in the pool and both clocked-source key
    /// shapes (pattern: `crates/smelt-db/tests/type_property_tests.rs::reachability`).
    #[test]
    fn reachability_sample_inhabits_every_pool_construct() {
        const N: usize = 200;
        let mut runner = TestRunner::deterministic();
        let strat = arb_recipe(RecipePool::partition_append_only());

        let mut seen_kinds: HashSet<&'static str> = HashSet::new();
        let mut seen_key_shapes: HashSet<KeyShape> = HashSet::new();
        for _ in 0..N {
            let recipe = strat.new_tree(&mut runner).unwrap().current();
            seen_kinds.insert(construct_kind_name(recipe.construct));
            seen_key_shapes.insert(recipe.source.key_shape);
        }

        for kind in [
            "pass_through",
            "filter",
            "additive_agg",
            "idempotent_agg",
            "decomposed_agg",
            "holistic_agg",
        ] {
            assert!(
                seen_kinds.contains(kind),
                "N={N} sample never generated construct {kind:?} — generator regression"
            );
        }
        assert!(
            seen_key_shapes.contains(&KeyShape::Single),
            "N={N} sample never generated KeyShape::Single"
        );
        assert!(
            seen_key_shapes.contains(&KeyShape::Composite),
            "N={N} sample never generated KeyShape::Composite"
        );
    }

    /// `recipe_names_its_matrix_cells` (plan Phase 1 TDD list): each pool
    /// construct maps to at least one coverage-matrix `(construct ×
    /// source-property)` cell id.
    #[test]
    fn recipe_names_its_matrix_cells() {
        let representatives = [
            BodyConstruct::PassThrough,
            BodyConstruct::Filter { threshold: 1 },
            BodyConstruct::AdditiveAgg,
            BodyConstruct::IdempotentAgg,
            BodyConstruct::DecomposedAgg,
            BodyConstruct::HolisticAgg,
        ];
        for construct in representatives {
            let cells = construct.matrix_cell_ids();
            assert!(
                !cells.is_empty(),
                "{construct:?} maps to zero coverage-matrix cells"
            );
            for cell in &cells {
                assert!(
                    cell.contains('×'),
                    "cell id {cell:?} is not of the form `construct × source-property`"
                );
            }
        }
    }

    /// `oracle_sql_is_model_body_with_sources_swapped` (plan Phase 1 TDD
    /// list): the rendered oracle query equals the model body with each
    /// `smelt.sources.<x>` replaced by its physical table name, nothing else
    /// changed.
    #[test]
    fn oracle_sql_is_model_body_with_sources_swapped() {
        let mut runner = TestRunner::deterministic();
        let strat = arb_recipe(RecipePool::partition_append_only());
        for _ in 0..50 {
            let recipe = strat.new_tree(&mut runner).unwrap().current();
            let body = render::render_model_body(&recipe);
            let oracle = render::render_oracle_sql(&recipe);
            let smelt_ref = format!("smelt.sources.{}", recipe.source.name);
            let physical_ref = format!("main.sources_{}", recipe.source.name);

            // Independent, round-trip check rather than re-deriving `expected`
            // via the same `.replace()` call `render_oracle_sql` itself uses
            // internally (which would only ever agree with its own logic):
            // the source-ref occurrence counts must match, the physical name
            // must appear in the oracle and never in the body, the smelt ref
            // must appear in the body and never survive into the oracle, and
            // substituting the physical name back to the smelt ref in the
            // oracle must reproduce the body exactly.
            assert!(
                body.contains(&smelt_ref),
                "model body must reference smelt.sources.{}",
                recipe.source.name
            );
            assert!(
                !body.contains(&physical_ref),
                "model body must not already contain the physical table name"
            );
            assert!(
                oracle.contains(&physical_ref),
                "oracle SQL must reference the physical table main.sources_{}",
                recipe.source.name
            );
            assert!(
                !oracle.contains(&smelt_ref),
                "oracle SQL must not retain the smelt.sources reference"
            );
            assert_eq!(
                body.matches(&smelt_ref).count(),
                oracle.matches(&physical_ref).count(),
                "every smelt.sources occurrence in the body must become exactly \
                 one physical-table occurrence in the oracle"
            );
            assert_eq!(
                oracle.replace(&physical_ref, &smelt_ref),
                body,
                "reversing the physical-table substitution must reproduce the \
                 model body exactly, with nothing else changed"
            );
        }
    }
}
