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

/// Which backend a staged Link-C project targets
/// (`docs/plans/20260720-prod-w9-spark-conformance-twin.md` Phase 2). Selects
/// both `render::render_smelt_yml_for`'s emitted target block and
/// `LinkCProject::run_with_target`'s backend factory arm. `DuckDb` is the
/// only variant ever constructed today — `SparkDelta` exists so the seam is
/// in place ahead of the Spark arm a later phase of that plan wires up;
/// selecting it from `run_with_target` is `unimplemented!()` until then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConformanceTarget {
    DuckDb,
    SparkDelta,
}

/// Dedicated Spark/Delta schema the generative conformance harness's Spark
/// twin stages every case into (`docs/plans/20260720-prod-w9-spark-conformance-twin.md`
/// Phase 3) — isolated from every other Spark integration test's own schema
/// (`spark_smoke.rs`'s `analytics`, `cross_engine_parity.rs`'s own schemas),
/// so a stale table from an unrelated suite can never collide with a
/// generated recipe's deterministic model/source names.
pub const SPARK_CONFORMANCE_SCHEMA: &str = "smelt_conf_gen";

/// The Spark Connect URL the conformance harness's Spark arms connect to:
/// `SPARK_CONNECT_URL` from the environment (`scripts/spark-env.sh`'s
/// convention, mirroring `crates/smelt-cli/tests/common/mod.rs::spark_connect_url`),
/// falling back to the default local Spark Connect port when unset. Callers
/// that reach the Spark arm at all have already checked
/// `SPARK_CONNECT_URL.is_some()` (the harness's skip-when-unset gate), so
/// this fallback is only ever exercised by a caller that bypassed that
/// check — it fails loud on connect rather than silently targeting nothing.
pub fn spark_connect_url() -> String {
    std::env::var("SPARK_CONNECT_URL").unwrap_or_else(|_| "sc://localhost:15002".to_string())
}

/// The Delta warehouse directory the conformance harness's Spark arms write
/// managed tables to: `SMELT_SPARK_WAREHOUSE` from the environment
/// (`scripts/spark-env.sh`'s convention) when set, otherwise a directory
/// sibling to `db_path` (this module's pre-Phase-3 fallback, kept for a
/// caller with no env var set).
pub fn spark_warehouse_dir(db_path: &std::path::Path) -> std::path::PathBuf {
    std::env::var("SMELT_SPARK_WAREHOUSE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            db_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("spark_warehouse")
        })
}

/// A source's mutation posture (`docs/plans/20260712-generative-maintenance-conformance.md`
/// Phase 4; design §6 "mixed models"). Phase 1-3's pool is exclusively
/// [`SourcePosture::AppendOnly`] (the fixed `events(d, id, val)` shape);
/// Phase 4's [`MutableEnrichedRecipe`] adds an unclocked
/// [`SourcePosture::MutableSnapshot`] dimension into the pool. Mirrors
/// `smelt_core::sources::MutationProfile`'s two testkit-relevant variants
/// (the `ChangeFeed` variant is out of this crate's generated pool).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourcePosture {
    /// Rows are only ever appended; an existing row never changes.
    AppendOnly,
    /// The table is a mutable snapshot: rows may be updated in place with no
    /// change history — the wire name `mutation_profile: mutable_snapshot`
    /// (`sources.md`).
    MutableSnapshot,
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
    /// ladder rung 1/3, `incremental_models.md` §"The algebraic maintenance
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
    /// Declared mutation posture (Phase 4). Every Phase 1-3 source is
    /// [`SourcePosture::AppendOnly`]; only [`SourceRecipe::mutable_dimension`]
    /// produces [`SourcePosture::MutableSnapshot`].
    pub posture: SourcePosture,
    /// A declared `key_recurrence` bound on this source
    /// (`docs/plans/20260715-composed-axes-conditional-maintenance.md`
    /// Phase A6; `sources.md` §"`mutation_profile` — the structured
    /// block"), rendered under the structured `mutation_profile:` block
    /// rather than the bare-string shorthand. `None` for every source
    /// built before Phase A6 (route 3's own declared-recurrence composed
    /// pool is the only consumer).
    pub key_recurrence: Option<KeyRecurrenceDecl>,
}

/// A declared `key_recurrence` bound (`sources.md` §"`mutation_profile` —
/// the structured block"): every pair of rows sharing `key` lies within
/// `window` of each other on the event-time axis. `window` is the raw
/// interval literal text (e.g. `"3 days"`), rendered verbatim into the
/// YAML.
#[derive(Debug, Clone)]
pub struct KeyRecurrenceDecl {
    pub key: Vec<String>,
    pub window: String,
}

impl SourceRecipe {
    /// The append-only, clocked `events(d, id, val)` source shape (`d`
    /// clocked/partition, `id` the row key, `val` the INTEGER payload).
    /// `pub(crate)` rather than private: [`crate::feed`]'s Phase 8
    /// `change_feed`-driven keyed recipe reuses this exact shape, declaring
    /// the resulting source as `change_feed` at staging time rather than
    /// through [`SourcePosture`] (see that module's doc comment for why).
    pub(crate) fn events(key_shape: KeyShape) -> Self {
        Self {
            name: "events".to_string(),
            clock_column: "d".to_string(),
            key_column: "id".to_string(),
            payload_column: "val".to_string(),
            key_shape,
            posture: SourcePosture::AppendOnly,
            key_recurrence: None,
        }
    }

    /// The append-only, clocked `events(d, id, val)` source ([`Self::events`]),
    /// additionally declaring a `key_recurrence` bound
    /// (`docs/plans/20260715-composed-axes-conditional-maintenance.md`
    /// Phase A6) — route 3's declared fallback
    /// (`incremental_models.md` §"Key temporal locality", route 3).
    pub(crate) fn events_with_key_recurrence(key: Vec<String>, window: &str) -> Self {
        let mut source = Self::events(KeyShape::Single);
        source.key_recurrence = Some(KeyRecurrenceDecl {
            key,
            window: window.to_string(),
        });
        source
    }

    /// An unclocked `mutable_snapshot` dimension source, keyed on `id` and
    /// carrying one mutable INTEGER attribute column `attr` (the
    /// `daily_events_enriched.sql`/`raw.users.user_name` role, narrowed to
    /// INTEGER so it keeps the same numeric-payload discipline — design §5 —
    /// as every other generated payload). Deliberately reuses the fact
    /// source's own key space rather than introducing a separate
    /// foreign-key column: [`MutableEnrichedRecipe`] joins the fact's own
    /// row key straight to this dimension's `id` (1:1), so
    /// [`crate::schedule_gen::GenRow`] /
    /// [`crate::s_tracker::STracker`]'s existing `events(d, id, val)` shape
    /// needs no widening for this phase — design §6 "mixed models" only
    /// requires *a* mutable dimension in the pool, not a fan-out join.
    pub fn mutable_dimension(name: &str) -> Self {
        Self {
            name: name.to_string(),
            clock_column: String::new(),
            key_column: "id".to_string(),
            payload_column: "attr".to_string(),
            key_shape: KeyShape::Single,
            posture: SourcePosture::MutableSnapshot,
            key_recurrence: None,
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

/// A definition-change edit a `RewriteModel` schedule step
/// (`crate::schedule_gen::ConformanceStep::RewriteModel`) can apply to an
/// already-staged recipe's model body
/// (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 9;
/// `incremental_models.md` §"The definition-change trigger"). Both variants
/// are deliberately narrow, hand-picked shapes — not a generated construct
/// pool — since Phase 9's scope is asserting TODAY's contract (model-hash
/// change invalidates the interval store; the run pipeline always compiles
/// and executes whatever SQL is currently on disk), not the spec's
/// `SkeletonAdd`/`PureBackfill`/`UpstreamRederive` classification, which is
/// unbuilt (no `derive_model_maintenance_plan` caller reads a prior
/// definition to classify an added column — confirmed by the same `rg`
/// sweep noted in this plan's "Deferred during implementation" section for
/// Phase 7's pin surface).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelEdit {
    /// Adds a derived, non-skeleton payload column computed from the
    /// existing payload column (e.g. `COUNT(*) AS row_count` alongside an
    /// aggregate's existing `SUM(val)`) — same `GROUP BY`/identity shape,
    /// so [`ModelRecipe::grain`]'s declared `unique_key` is unchanged.
    AddPayloadColumn,
    /// Adds the source's row-key column into the aggregate's `GROUP BY` (a
    /// skeleton/identity position) — a grain change:
    /// `incremental_models.md`'s `SkeletonAdd` territory. Only meaningful for
    /// the aggregate constructs (`AdditiveAgg`/`IdempotentAgg`/
    /// `DecomposedAgg`/`HolisticAgg`), which have a `GROUP BY` skeleton to
    /// widen; the row-shaped constructs (`PassThrough`/`Filter`) already
    /// project every source column and have none.
    AddGroupingColumn,
}

/// The [`ModelEdit`]s meaningful for `construct` — [`ModelRecipe::evolution`]'s
/// value, and the set [`crate::render::render_model_body_with_edit`] must
/// handle for that construct.
fn applicable_evolutions(construct: BodyConstruct) -> Vec<ModelEdit> {
    match construct {
        BodyConstruct::PassThrough | BodyConstruct::Filter { .. } => {
            vec![ModelEdit::AddPayloadColumn]
        }
        BodyConstruct::AdditiveAgg
        | BodyConstruct::IdempotentAgg
        | BodyConstruct::DecomposedAgg
        | BodyConstruct::HolisticAgg => {
            vec![ModelEdit::AddPayloadColumn, ModelEdit::AddGroupingColumn]
        }
    }
}

/// A fully-typed model recipe: one source, one body construct, one grain
/// declaration, ready for [`crate::render`] to turn into SQL/YAML text.
#[derive(Debug, Clone)]
pub struct ModelRecipe {
    pub model_name: String,
    pub source: SourceRecipe,
    pub grain: GrainDecl,
    pub construct: BodyConstruct,
    /// The [`ModelEdit`]s a `RewriteModel` schedule step may apply to this
    /// recipe (Phase 9) — empty for constructs with no meaningful edit.
    pub evolution: Vec<ModelEdit>,
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
            evolution: applicable_evolutions(construct),
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

/// Adversarial leaf constructs (`docs/plans/20260712-generative-maintenance-conformance.md`
/// Phase 2's "Implementation shape"): each deliberately defeats one of
/// `model_properties.md`'s fail-closed proofs rather than merely being an
/// unusual-but-provable shape. Kept as a type *separate* from
/// [`BodyConstruct`] — [`BodyConstruct`]'s only renderer
/// (`render::render_model_body`) is an exhaustive match, and `render.rs` is
/// outside Phase 2's edit scope (Critical files: `verdict.rs` new,
/// `recipe.rs` for the adversarial pool) — so [`AdversarialLeafRecipe`]
/// renders itself instead of routing through that match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdversarialLeaf {
    /// An opaque/unrecognised function call wraps the projected event-time
    /// column. `model_properties.md`'s event-time trace classifies an
    /// unrecognised function call as `Undecidable` — fail-closed, never an
    /// optimistic pass.
    OpaqueEventTime,
    /// `INTERSECT` combines two arms of the same source.
    /// `model_properties.md` §Known Divergences: set-operation distribution
    /// covers `UNION ALL` only; `derive_column_groups` fails closed on any
    /// other set operation, collapsing every payload column into one group
    /// sensitive to every declared source (`incremental_models.md` §Known
    /// Divergences' `INTERSECT`/`EXCEPT` entry).
    IntersectBody,
    /// `RANDOM()` occupies a skeleton (identity/dedup-key) position — a
    /// row-nondeterministic function feeding the column the plan needs a
    /// stable identity from.
    NondeterministicSkeleton,
    /// The projected event-time column is shifted by a calendar-variable
    /// (`INTERVAL '1 month'`) offset — `Offset::Symbolic`, which cannot
    /// populate a `Bounded{before, after}` scan window, forcing
    /// `NotDerivable` for the source rather than an approximate fixed-day
    /// guess (`model_properties.md`'s interval-literal parsing note).
    SymbolicIntervalBound,
}

/// Stable, human-readable identifier for an [`AdversarialLeaf`] — mirrors
/// [`construct_kind_name`]'s role for [`BodyConstruct`].
fn adversarial_leaf_name(leaf: AdversarialLeaf) -> &'static str {
    match leaf {
        AdversarialLeaf::OpaqueEventTime => "opaque_event_time",
        AdversarialLeaf::IntersectBody => "intersect_body",
        AdversarialLeaf::NondeterministicSkeleton => "nondeterministic_skeleton",
        AdversarialLeaf::SymbolicIntervalBound => "symbolic_interval_bound",
    }
}

/// A fully-typed adversarial recipe: one [`SourceRecipe`] (always the
/// append-only `events` shape, `KeyShape::Single`) paired with one
/// [`AdversarialLeaf`]. Self-rendering (see [`AdversarialLeaf`]'s doc
/// comment for why): [`Self::model_body`]/[`Self::model_file`]/
/// [`Self::source_yaml`] produce the same artifacts
/// [`crate::render::render_model_body`]/[`crate::render::render_model_file`]/
/// [`crate::render::render_source_yaml`] do for [`ModelRecipe`], without
/// routing through `render.rs`.
#[derive(Debug, Clone)]
pub struct AdversarialLeafRecipe {
    pub model_name: String,
    pub source: SourceRecipe,
    pub leaf: AdversarialLeaf,
}

impl AdversarialLeafRecipe {
    fn new(leaf: AdversarialLeaf) -> Self {
        Self {
            model_name: format!("adversarial_{}", adversarial_leaf_name(leaf)),
            source: SourceRecipe::events(KeyShape::Single),
            leaf,
        }
    }

    /// The declared `batched.unique_key` for this leaf's body shape: the
    /// source's own row key, plus the nondeterministic `tag` column for
    /// [`AdversarialLeaf::NondeterministicSkeleton`] (the whole point of
    /// that leaf is putting the nondeterministic function in a skeleton/
    /// identity position).
    pub fn unique_key(&self) -> Vec<String> {
        match self.leaf {
            AdversarialLeaf::NondeterministicSkeleton => {
                vec![self.source.key_column.clone(), "tag".to_string()]
            }
            _ => vec![self.source.key_column.clone()],
        }
    }

    /// The coverage-matrix cell id this leaf inhabits
    /// (`construct × source-property`, matching [`BodyConstruct::matrix_cell_ids`]'s
    /// convention).
    pub fn matrix_cell_id(&self) -> String {
        format!("{}×adversarial", adversarial_leaf_name(self.leaf))
    }

    /// The model's `SELECT` body — no frontmatter, mirroring
    /// [`crate::render::render_model_body`]'s contract for [`ModelRecipe`].
    pub fn model_body(&self) -> String {
        let src = format!("smelt.sources.{}", self.source.name);
        let d = &self.source.clock_column;
        let id = &self.source.key_column;
        let val = &self.source.payload_column;
        match self.leaf {
            AdversarialLeaf::OpaqueEventTime => {
                format!("SELECT smelt_testkit_opaque_udf({d}) AS {d}, {id}, {val} FROM {src}")
            }
            AdversarialLeaf::IntersectBody => {
                format!(
                    "SELECT {d}, {id}, {val} FROM {src} \
                     INTERSECT \
                     SELECT {d}, {id}, {val} FROM {src}"
                )
            }
            AdversarialLeaf::NondeterministicSkeleton => {
                format!("SELECT {d}, {id}, {val}, RANDOM() AS tag FROM {src}")
            }
            AdversarialLeaf::SymbolicIntervalBound => {
                format!("SELECT {d} + INTERVAL '1 month' AS {d}, {id}, {val} FROM {src}")
            }
        }
    }

    /// The full model file contents: frontmatter (`timeseries:` + `refresh:
    /// incremental` + `grain: partition`) followed by [`Self::model_body`] —
    /// the same shape [`crate::render::render_model_file`] produces for
    /// [`ModelRecipe`]. The retired `batched.unique_key` sub-block this used
    /// to carry [`Self::unique_key`] under is gone — it never fed
    /// row-identity derivation for a `Grain::Partition` output anyway
    /// (`derive::ModelInputs::declared_unique_key` is empty for every
    /// `Grain::Partition`), so dropping it changes no derived maintenance
    /// plan.
    pub fn model_file(&self) -> String {
        let d = &self.source.clock_column;
        format!(
            "---\ntimeseries:\n  event_time_column: {d}\n  partition_column: {d}\n  granularity: day\nrefresh: incremental\ngrain: partition\n---\n{body}\n",
            body = self.model_body(),
        )
    }

    /// The source YAML sidecar — same append-only `events(d, id, val)`
    /// shape [`crate::render::render_source_yaml`] renders for
    /// [`ModelRecipe`].
    pub fn source_yaml(&self) -> String {
        format!(
            "description: adversarial-leaf conformance source.\nmutation_profile: append_only\ntimeseries:\n  event_time_column: {d}\n  partition_column: {d}\n  granularity: day\ncolumns:\n  - name: {d}\n    type: DATE\n  - name: {id}\n    type: INTEGER\n  - name: {val}\n    type: INTEGER\n",
            d = self.source.clock_column,
            id = self.source.key_column,
            val = self.source.payload_column,
        )
    }
}

/// A `Strategy` drawing uniformly from the four named [`AdversarialLeaf`]
/// kinds (plan Phase 2 TDD list: "proptest over the adversarial pool").
pub fn arb_adversarial_leaf() -> impl Strategy<Value = AdversarialLeaf> {
    prop_oneof![
        Just(AdversarialLeaf::OpaqueEventTime),
        Just(AdversarialLeaf::IntersectBody),
        Just(AdversarialLeaf::NondeterministicSkeleton),
        Just(AdversarialLeaf::SymbolicIntervalBound),
    ]
}

/// The typed generator over [`AdversarialLeafRecipe`] — the adversarial-pool
/// counterpart of [`arb_recipe`].
pub fn arb_adversarial_recipe() -> impl Strategy<Value = AdversarialLeafRecipe> {
    arb_adversarial_leaf().prop_map(AdversarialLeafRecipe::new)
}

/// The fact+mutable-dimension enrichment recipe
/// (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 4;
/// design §6 "mixed models"; the
/// `examples/timeseries/models/daily_events_enriched.sql` shape): one
/// append-only clocked fact source (`events(d, id, val)`) joined 1:1 to one
/// unclocked `mutable_snapshot` dimension source
/// ([`SourceRecipe::mutable_dimension`]), whose key is read in the join's
/// `ON` predicate (a row-admission read) and whose `attr` feeds the select
/// list. Per `incremental_models.md` §"The plan matrix" the ON-read
/// makes the dimension-sourced `attr` column group membership-sensitive, so
/// the derived plan carries an `UpstreamMutation(dim)` cell assigned
/// `Technique::DeleteInsert` (the recompute family), never
/// `Technique::ColumnScopedMerge` — mirroring
/// `gate.rs::keyed_enriched_recipe_admits_membership_recompute`'s outcome
/// for the same join shape. Self-renders like [`AdversarialLeafRecipe`] —
/// the join body sits outside [`BodyConstruct`]'s exhaustive match, and
/// `render.rs` is outside this phase's edit scope (plan Critical files).
#[derive(Debug, Clone)]
pub struct MutableEnrichedRecipe {
    pub model_name: String,
    pub fact: SourceRecipe,
    pub dimension: SourceRecipe,
}

impl Default for MutableEnrichedRecipe {
    fn default() -> Self {
        Self::new()
    }
}

impl MutableEnrichedRecipe {
    /// The pool's one fixed mixed-model shape (design §6's "mixed models"
    /// needs exactly one mutable-dimension shape reachable; schedule
    /// generation — not model-shape generation — is this pool's generative
    /// surface, per the plan's Phase 4 Implementation shape).
    pub fn new() -> Self {
        Self {
            model_name: "recipe_mutable_enriched".to_string(),
            fact: SourceRecipe::events(KeyShape::Single),
            dimension: SourceRecipe::mutable_dimension("dim"),
        }
    }

    /// The declared `batched.unique_key`: the fact's own row key — still
    /// uniquely identifies each output row since the join is 1:1
    /// (`SourceRecipe::mutable_dimension`'s doc comment).
    pub fn unique_key(&self) -> Vec<String> {
        vec![self.fact.key_column.clone()]
    }

    /// The coverage-matrix cell id this recipe inhabits (mirrors
    /// [`BodyConstruct::matrix_cell_ids`]'s convention).
    pub fn matrix_cell_id(&self) -> String {
        "mutable_enriched×mixed".to_string()
    }

    /// The model's `SELECT` body: the fact source passed through, enriched
    /// by the dimension's `attr` column, joined on the fact's own row key.
    pub fn model_body(&self) -> String {
        let fact_src = format!("smelt.sources.{}", self.fact.name);
        let dim_src = format!("smelt.sources.{}", self.dimension.name);
        let d = &self.fact.clock_column;
        let id = &self.fact.key_column;
        let val = &self.fact.payload_column;
        let dim_id = &self.dimension.key_column;
        let attr = &self.dimension.payload_column;
        format!(
            "SELECT f.{d} AS {d}, f.{id} AS {id}, f.{val} AS {val}, dim.{attr} AS {attr} \
             FROM {fact_src} f JOIN {dim_src} dim ON f.{id} = dim.{dim_id}"
        )
    }

    /// The full model file contents: frontmatter (`timeseries:` + `refresh:
    /// incremental` + `grain: partition` +
    /// `maintenance.scan_bounds.per_source.<dim>.allow_full_scan: true`,
    /// mirroring `daily_events_enriched.sql`'s own frontmatter) followed by
    /// [`Self::model_body`]. The retired `batched.unique_key` sub-block this
    /// used to carry [`Self::unique_key`] under is gone — it never fed
    /// row-identity derivation for a `Grain::Partition` output anyway
    /// (`derive::ModelInputs::declared_unique_key` is empty for every
    /// `Grain::Partition`), so dropping it changes no derived maintenance
    /// plan.
    pub fn model_file(&self) -> String {
        let d = &self.fact.clock_column;
        format!(
            "---\ntimeseries:\n  event_time_column: {d}\n  partition_column: {d}\n  granularity: day\nrefresh: incremental\ngrain: partition\nmaintenance:\n  scan_bounds:\n    per_source:\n      {dim_name}:\n        allow_full_scan: true\n---\n{body}\n",
            dim_name = self.dimension.name,
            body = self.model_body(),
        )
    }

    /// The fact source YAML sidecar — the same append-only `events(d, id,
    /// val)` shape [`crate::render::render_source_yaml`] renders for
    /// [`ModelRecipe`].
    pub fn fact_source_yaml(&self) -> String {
        format!(
            "description: generative-conformance mixed-pool fact source.\nmutation_profile: append_only\ntimeseries:\n  event_time_column: {d}\n  partition_column: {d}\n  granularity: day\ncolumns:\n  - name: {d}\n    type: DATE\n  - name: {id}\n    type: INTEGER\n  - name: {val}\n    type: INTEGER\n",
            d = self.fact.clock_column,
            id = self.fact.key_column,
            val = self.fact.payload_column,
        )
    }

    /// The dimension source YAML sidecar — unclocked,
    /// `mutation_profile: mutable_snapshot` (`sources.md`).
    pub fn dimension_source_yaml(&self) -> String {
        format!(
            "description: generative-conformance mixed-pool mutable dimension.\nmutation_profile: mutable_snapshot\ncolumns:\n  - name: {id}\n    type: INTEGER\n  - name: {attr}\n    type: INTEGER\n",
            id = self.dimension.key_column,
            attr = self.dimension.payload_column,
        )
    }

    /// The oracle query for this recipe (design §6 "mixed models": "the
    /// S-restriction applies to the driving source; the dimension
    /// contributes its current state"): [`Self::model_body`] with the fact
    /// source's `smelt.sources.*` reference swapped for `fact_table_ref`
    /// (either the physical fact table, for a full-refresh oracle, or an
    /// `STracker`-materialized `S_k` temp table) and the dimension's
    /// reference always swapped for its CURRENT physical table — the
    /// dimension is never S-restricted.
    pub fn oracle_body_over(&self, fact_table_ref: &str) -> String {
        self.model_body()
            .replace(&format!("smelt.sources.{}", self.fact.name), fact_table_ref)
            .replace(
                &format!("smelt.sources.{}", self.dimension.name),
                &format!("main.sources_{}", self.dimension.name),
            )
    }
}

/// The closure-pruned column-scoped-`MERGE` recipe
/// (`docs/plans/20260809-sensitivity-precision.md` Phase 5): the same
/// `grain: partition` fact+dimension shape as [`MutableEnrichedRecipe`]
/// (the `examples/timeseries/models/daily_events_enriched.sql` MP11
/// column-scoped-`MERGE` mechanism) EXCEPT the fact/dimension join is a
/// `LEFT JOIN` and the dimension declares its own `unique_key`. Those two
/// facts are exactly the two conjuncts
/// `crates/smelt-logical/src/analysis/skeleton_closure.rs`'s
/// `skeleton_source_closure` needs to return `Closed` without any
/// `referential_integrity` world-fact (which the closure-pruned membership
/// pass never consults, `grouping.rs`'s own doc comment: conjunct 3
/// one-to-one via the dimension's declared `unique_key`, conjunct 4
/// row-preservation via the `LEFT JOIN` shape itself, unconditionally).
/// Unlike [`MutableEnrichedRecipe`] (bare INNER `JOIN`, no declared
/// dimension `unique_key`, membership-sensitive, `Technique::DeleteInsert`)
/// and `KeyedEnrichedRecipe` in `gate.rs` (INNER JOIN, dimension read only
/// in `ON`, never selected), this recipe SELECTS the dimension's own
/// `attr` column directly through a closed `LEFT JOIN` — the one shape the
/// closure proof can actually prune, so the `{attr}` column group's
/// `UpstreamMutation(dim)` cell derives `Technique::ColumnScopedMerge`
/// instead of falling back to the recompute family (mirrors
/// `smelt-logical/tests/maintenance_tracer.rs::closed_outer_enrichment_join_upstream_mutation_derives_column_scoped_merge`'s
/// hand-built `ModelInputs`, staged here through the real
/// disk-backed/Salsa-backed derivation and the real `execute_project`
/// pipeline).
#[derive(Debug, Clone)]
pub struct ValueEnrichedRecipe {
    pub model_name: String,
    pub fact: SourceRecipe,
    pub dimension: SourceRecipe,
}

impl Default for ValueEnrichedRecipe {
    fn default() -> Self {
        Self::new()
    }
}

impl ValueEnrichedRecipe {
    /// The pool's one fixed shape (mirrors [`MutableEnrichedRecipe::new`]'s
    /// own doc comment: exactly one closure-pruned enrichment shape needs
    /// to be reachable, not a generated construct family).
    pub fn new() -> Self {
        Self {
            model_name: "recipe_value_enriched".to_string(),
            fact: SourceRecipe::events(KeyShape::Single),
            dimension: SourceRecipe::mutable_dimension("value_enrich_dim"),
        }
    }

    /// The declared `batched.unique_key`: the fact's own row key — still
    /// uniquely identifies each output row since the join is 1:1
    /// ([`SourceRecipe::mutable_dimension`]'s doc comment).
    pub fn unique_key(&self) -> Vec<String> {
        vec![self.fact.key_column.clone()]
    }

    /// The model's `SELECT` body: the fact source passed through, enriched
    /// by the dimension's `attr` column via a `LEFT JOIN` on the fact's own
    /// row key — the dimension's `attr` is a SELECTED payload column, not
    /// merely read in the join's `ON` predicate (unlike `KeyedEnrichedRecipe`
    /// in `gate.rs`, whose whole point is the opposite shape). Otherwise
    /// identical to [`MutableEnrichedRecipe::model_body`] with `JOIN`
    /// swapped for `LEFT JOIN`.
    pub fn model_body(&self) -> String {
        let fact_src = format!("smelt.sources.{}", self.fact.name);
        let dim_src = format!("smelt.sources.{}", self.dimension.name);
        let d = &self.fact.clock_column;
        let id = &self.fact.key_column;
        let val = &self.fact.payload_column;
        let dim_id = &self.dimension.key_column;
        let attr = &self.dimension.payload_column;
        format!(
            "SELECT f.{d} AS {d}, f.{id} AS {id}, f.{val} AS {val}, dim.{attr} AS {attr} \
             FROM {fact_src} f LEFT JOIN {dim_src} dim ON f.{id} = dim.{dim_id}"
        )
    }

    /// The full model file: `timeseries:` + `refresh: incremental` +
    /// `grain: partition` frontmatter (mirroring
    /// [`MutableEnrichedRecipe::model_file`]) plus the dimension declared
    /// `allow_full_scan` (its `ColumnScopedMerge` cell's admission
    /// precondition). The column-scoped `MERGE`'s own `ON`-predicate key
    /// (`decide_column_merge_dispatch`'s `model_declares_unique_key`
    /// precondition, `smelt_core::PartitionGrainConfig::unique_key`) is NOT
    /// declarable in SQL frontmatter — the `batched:` sub-block there was
    /// retired in favour of the top-level `unique_key:` identity fact, which
    /// instead flips the DERIVED grain to `Key`/`KeyPerPartition`
    /// (`smelt_core::config::derive_grain`), conflicting with this recipe's
    /// asserted `grain: partition`. The only remaining surface for a
    /// partition-grain `PartitionGrainConfig.unique_key` is smelt.yml's
    /// `models.<name>.batched.unique_key` (`ModelConfig::batched`,
    /// `Config::get_incremental_with_metadata`'s smelt.yml-only fallback
    /// arm) — the staging harness (`gate.rs::stage_value_enriched_recipe`)
    /// writes that block into the generated `smelt.yml` rather than here.
    pub fn model_file(&self) -> String {
        let d = &self.fact.clock_column;
        format!(
            "---\ntimeseries:\n  event_time_column: {d}\n  partition_column: {d}\n  granularity: day\nrefresh: incremental\ngrain: partition\nmaintenance:\n  scan_bounds:\n    per_source:\n      {dim_name}:\n        allow_full_scan: true\n---\n{body}\n",
            dim_name = self.dimension.name,
            body = self.model_body(),
        )
    }

    /// The fact source YAML sidecar — the same append-only `events(d, id,
    /// val)` shape [`MutableEnrichedRecipe::fact_source_yaml`] renders.
    pub fn fact_source_yaml(&self) -> String {
        format!(
            "description: generative-conformance closure-pruned-enrichment fact source.\n\
             mutation_profile: append_only\ntimeseries:\n  event_time_column: {d}\n  \
             partition_column: {d}\n  granularity: day\ncolumns:\n  - name: {d}\n    type: \
             DATE\n  - name: {id}\n    type: INTEGER\n  - name: {val}\n    type: INTEGER\n",
            d = self.fact.clock_column,
            id = self.fact.key_column,
            val = self.fact.payload_column,
        )
    }

    /// The dimension source YAML sidecar: unclocked,
    /// `mutation_profile: mutable_snapshot`, WITH a declared `unique_key:`
    /// (`sources.md` §"Row identity") — unlike
    /// [`MutableEnrichedRecipe::dimension_source_yaml`], this is the one
    /// fact the closure proof's one-to-one conjunct actually needs to prune
    /// the LEFT JOIN's own `ON` read.
    pub fn dimension_source_yaml(&self) -> String {
        format!(
            "description: generative-conformance closure-pruned-enrichment mutable dimension.\nmutation_profile: mutable_snapshot\nunique_key: [{id}]\ncolumns:\n  - name: {id}\n    type: INTEGER\n  - name: {attr}\n    type: INTEGER\n",
            id = self.dimension.key_column,
            attr = self.dimension.payload_column,
        )
    }

    /// The oracle query for this recipe: [`Self::model_body`] with the fact
    /// source reference swapped for `fact_table_ref` (a full-refresh oracle
    /// or an `STracker`-materialized `S_k` temp table) and the dimension's
    /// reference swapped for its CURRENT physical table.
    pub fn oracle_body_over(&self, fact_table_ref: &str) -> String {
        self.model_body()
            .replace(&format!("smelt.sources.{}", self.fact.name), fact_table_ref)
            .replace(
                &format!("smelt.sources.{}", self.dimension.name),
                &format!("main.sources_{}", self.dimension.name),
            )
    }
}

/// A source's declared `batched.unique_key`/source-YAML rendering, factored
/// out of [`SourceRecipe`] so [`KeyedRecipe`] (which has no `GrainDecl` —
/// keyed output declares no `timeseries:`/`unique_key`, `incremental_models.md`
/// §Known Divergences "The key grain") can render its driving source's YAML the same way
/// [`crate::render::render_source_yaml`] does for a [`ModelRecipe`], without
/// requiring a `GrainDecl` to exist.
impl SourceRecipe {
    pub fn source_yaml(&self) -> String {
        match self.posture {
            SourcePosture::AppendOnly => {
                let mutation_profile = match &self.key_recurrence {
                    // Structured block (`sources.md` §"`mutation_profile` —
                    // the structured block"): the bare-string shorthand has
                    // no room for a nested `key_recurrence:` sub-fact.
                    Some(kr) => format!(
                        "mutation_profile:\n  kind: append_only\n  key_recurrence:\n    key: [{}]\n    window: '{}'\n",
                        kr.key.join(", "),
                        kr.window,
                    ),
                    None => "mutation_profile: append_only\n".to_string(),
                };
                format!(
                    "description: generative-conformance keyed driving source.\n{mutation_profile}timeseries:\n  event_time_column: {d}\n  partition_column: {d}\n  granularity: day\ncolumns:\n  - name: {d}\n    type: DATE\n  - name: {id}\n    type: INTEGER\n  - name: {val}\n    type: INTEGER\n",
                    d = self.clock_column,
                    id = self.key_column,
                    val = self.payload_column,
                )
            }
            SourcePosture::MutableSnapshot => format!(
                "description: generative-conformance keyed unclocked source.\nmutation_profile: mutable_snapshot\ncolumns:\n  - name: {id}\n    type: INTEGER\n  - name: {val}\n    type: INTEGER\n",
                id = self.key_column,
                val = self.payload_column,
            ),
        }
    }
}

/// The `grain: key` pool's combiner family
/// (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 5;
/// `incremental_models.md` §"The algebraic maintenance ladder"): both are
/// direct-monoid, admitted by the built classifier seed
/// (`crates/smelt-logical/src/rules/cumulative.rs`'s aggregator allowlist —
/// `incremental_models.md` §Known Divergences "The key grain": "the classifier covers only the
/// direct-monoid families").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyedCombiner {
    /// `SUM(val)` — an invertible commutative group (ladder rung 3). The
    /// `Grade::Additive` reconciliation-ledger family: entries record delta
    /// identities and a repeat fold of an already-processed window is
    /// refused (`KeyedReprocessedWindow`, `incremental_models.md` §"Reprocessing").
    Additive,
    /// `MAX(val)` — an idempotent, non-invertible monoid (ladder rung 1,
    /// not a group). The `Grade::Idempotent` family: entries record only a
    /// frontier watermark, so re-folding a window is harmless.
    Idempotent,
}

impl KeyedCombiner {
    pub fn kind_name(self) -> &'static str {
        match self {
            KeyedCombiner::Additive => "additive",
            KeyedCombiner::Idempotent => "idempotent",
        }
    }

    /// `(aggregate function, output column alias)` this combiner projects.
    pub fn agg_and_alias(self) -> (&'static str, &'static str) {
        match self {
            KeyedCombiner::Additive => ("SUM", "total"),
            KeyedCombiner::Idempotent => ("MAX", "max_val"),
        }
    }
}

/// A `Strategy` drawing uniformly from the two [`KeyedCombiner`] families.
pub fn arb_keyed_combiner() -> impl Strategy<Value = KeyedCombiner> {
    prop_oneof![
        Just(KeyedCombiner::Additive),
        Just(KeyedCombiner::Idempotent),
    ]
}

/// A `grain: key` recipe (Phase 5): `SELECT <key>, <agg>(<val>) AS <alias>
/// FROM smelt.sources.<name> GROUP BY <key>` over one [`SourceRecipe`].
/// [`Self::new_window_forward`] uses the clocked append-only `events` shape
/// (the run-shape derivation's window-forward posture,
/// `incremental_models.md` §"The two run shapes"); [`Self::new_snapshot_reconcile`]
/// uses the unclocked `mutable_snapshot` dimension shape (selecting the
/// snapshot-reconcile posture, refused today — `incremental_models.md` §Known
/// Divergences "The key grain": "the snapshot-reconcile executor is unbuilt").
#[derive(Debug, Clone)]
pub struct KeyedRecipe {
    pub model_name: String,
    pub source: SourceRecipe,
    pub combiner: KeyedCombiner,
}

impl KeyedRecipe {
    pub fn new_window_forward(combiner: KeyedCombiner) -> Self {
        Self {
            model_name: format!("recipe_keyed_{}", combiner.kind_name()),
            source: SourceRecipe::events(KeyShape::Single),
            combiner,
        }
    }

    pub fn new_snapshot_reconcile(combiner: KeyedCombiner) -> Self {
        Self {
            model_name: format!("recipe_keyed_snapshot_{}", combiner.kind_name()),
            source: SourceRecipe::mutable_dimension("keyed_snapshot_dim"),
            combiner,
        }
    }
}

/// One `[start, end)` window of a generated keyed schedule, plus the rows
/// landing in it — the keyed pool's analogue of
/// [`crate::schedule_gen::ConformanceStep::RunWindow`], with no late-row/
/// re-run step kinds (redelivery is a dedicated probe, not a generated
/// schedule shape — plan Phase 5 TDD list
/// `redelivered_window_refuses_for_additive_keyed`).
#[derive(Debug, Clone)]
pub struct KeyedRunWindow {
    pub start: chrono::NaiveDate,
    pub end: chrono::NaiveDate,
    pub rows: Vec<crate::schedule_gen::GenRow>,
}

/// A generated sequence of [`KeyedRunWindow`]s.
#[derive(Debug, Clone)]
pub struct KeyedSchedule(pub Vec<KeyedRunWindow>);

/// The key every generated window deliberately re-touches (design §5
/// "Key-recurrence control": "keyed recipes generate schedules with
/// deliberate key re-touch across windows — the interesting case for
/// merges").
const KEYED_SHARED_KEY_ID: i64 = 1;

/// Schema-generic keyed-pool schedule generator: 2-3 disjoint one-day
/// windows; every window contributes one row keyed on
/// [`KEYED_SHARED_KEY_ID`] (guaranteeing key re-touch across windows) plus
/// 1-2 rows keyed on fresh ids (variety). Never re-runs a window — a
/// re-delivered window is the dedicated probe's own hand-built schedule, not
/// a generated shape here.
pub fn arb_keyed_schedule() -> impl Strategy<Value = KeyedSchedule> {
    let base = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid base date");
    (2_usize..=3).prop_flat_map(move |n_windows| {
        proptest::collection::vec(
            proptest::collection::vec(arb_payload_value(), 1..=2),
            n_windows,
        )
        .prop_map(move |extra_vals| build_keyed_schedule(base, &extra_vals))
    })
}

fn build_keyed_schedule(base: chrono::NaiveDate, extra_vals: &[Vec<i64>]) -> KeyedSchedule {
    use crate::schedule_gen::GenRow;

    let mut windows = Vec::new();
    let mut next_id = 100_i64;
    for (i, vals) in extra_vals.iter().enumerate() {
        let start = base + chrono::Duration::days(i as i64);
        let end = start + chrono::Duration::days(1);

        let mut rows = vec![GenRow {
            d: start,
            id: KEYED_SHARED_KEY_ID,
            val: 1 + i as i64,
        }];
        for val in vals {
            rows.push(GenRow {
                d: start,
                id: next_id,
                val: *val,
            });
            next_id += 1;
        }
        windows.push(KeyedRunWindow { start, end, rows });
    }
    KeyedSchedule(windows)
}

/// A `Strategy` producing `n` pairwise-distinct `i64` ordering-key values by
/// construction (design §5 "Key-recurrence control": "where ordering-
/// sensitive combiners (`MAX_BY`-family) are generated, ordering keys are
/// made unique by construction so the documented ties carve-out cannot fire
/// spuriously" — `incremental_models.md` §"Ordering ties"). The order-monotone
/// overwrite combiner family this discipline targets is not yet an admitted
/// technique (`incremental_models.md` §Known Divergences "The key grain": "the classifier union
/// (overwrite, once-write, and plain-overwrite families) ... are unbuilt"),
/// so this generator is not wired into [`KeyedCombiner`] today — but the
/// discipline it must uphold once that family lands is independently
/// testable now: a strictly increasing sequence can never collide, by
/// construction rather than by (unprovable) statistical luck.
pub fn arb_unique_ordering_keys(n: usize) -> impl Strategy<Value = Vec<i64>> {
    (0..1_000_000_i64).prop_map(move |base| (0..n as i64).map(|i| base + i).collect())
}

// ---------------------------------------------------------------------
// Phase A6: the composed (`grain: key` + `timeseries:`) recipe family,
// covering all three key-temporal-locality routes
// (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase A6;
// `incremental_models.md` §"Key temporal locality").
// ---------------------------------------------------------------------

/// The three key-temporal-locality routes (`incremental_models.md` §"Key
/// temporal locality") a composed recipe may establish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposedRoute {
    /// Route 1 (key-embedded): `partition_column` is itself a `unique_key`
    /// column.
    KeyEmbedded,
    /// Route 2 (key-determined): the partition projection is a per-key
    /// constant under a declared `functional_dependencies:` entry.
    KeyDetermined,
    /// Route 3 (recurrence-bounded, declared): a declared `key_recurrence`
    /// bound `r` on the driving source, admitted checked.
    RecurrenceBounded,
}

/// Stable, human-readable identifier for a [`ComposedRoute`] — mirrors
/// [`construct_kind_name`]'s role for [`BodyConstruct`].
pub fn composed_route_name(route: ComposedRoute) -> &'static str {
    match route {
        ComposedRoute::KeyEmbedded => "key_embedded",
        ComposedRoute::KeyDetermined => "key_determined",
        ComposedRoute::RecurrenceBounded => "recurrence_bounded",
    }
}

/// A `Strategy` drawing uniformly from the three [`ComposedRoute`]s.
pub fn arb_composed_route() -> impl Strategy<Value = ComposedRoute> {
    prop_oneof![
        Just(ComposedRoute::KeyEmbedded),
        Just(ComposedRoute::KeyDetermined),
        Just(ComposedRoute::RecurrenceBounded),
    ]
}

/// The declared recurrence-bound window route 3's recipes use — matches
/// `crates/smelt-runtime/tests/locality_route3_recurrence_check.rs`'s own
/// flagship shape (`r = 3 days`).
pub const ROUTE3_RECURRENCE_WINDOW: &str = "3 days";

/// The shared "storm" key every generated route-3 schedule redelivers
/// across every window (`arb_composed_route3_schedule`) — the redelivery-
/// storm hazard the recipe family is scoped to cover.
pub const ROUTE3_STORM_KEY_ID: i64 = 900;

/// A composed (`grain: key` + `timeseries:`) recipe covering one of the
/// three key-temporal-locality routes. Every route uses the same clocked,
/// append-only `events(d, id, val)` source; only the body shape, declared
/// `unique_key`, and partition-column provenance differ:
///
/// - [`ComposedRoute::KeyEmbedded`]: `SELECT id, d, SUM(val) AS total FROM
///   events GROUP BY id, d` — `partition_column` (`d`) is itself a
///   `unique_key` column (route 1). Fully executable through the real
///   `execute_project` pipeline — no extremal aggregate involved, so it
///   does not hit the nullability blocker described below.
/// - [`ComposedRoute::KeyDetermined`]: `SELECT id, CAST(d AS DATE) AS
///   pdate, SUM(val) AS total FROM events GROUP BY id`, with a declared
///   `functional_dependencies: [{key: [id], determines: pdate}]` — `pdate`
///   is a direct scalar wrapper of the driving source's own clock column,
///   the one NOT-NULL-provable, non-extremal shape route 2 admits
///   (`smelt_logical::analysis::not_null::partition_column_provably_not_null`'s
///   doc comment).
/// - [`ComposedRoute::RecurrenceBounded`]: `SELECT id, MAX(d) AS last_seen
///   FROM events GROUP BY id`, with the driving source declaring
///   `key_recurrence: {key: [id], window: '3 days'}` — the flagship
///   extremal-fold shape route 3 exists for.
///
/// `KeyDetermined`'s and `RecurrenceBounded`'s rendered model+source files
/// are admitted by the real key-temporal-locality gate
/// (`establish_locality`, exercised through `smelt-db`'s real
/// `maintenance_plan_report` Salsa query over the real staged
/// frontmatter/YAML) but are **not** executable through the real
/// `execute_project` pipeline today — a documented, pre-existing gap
/// independent of this pool (`incremental_models.md` §Known Divergences:
/// every extremal `MIN`/`MAX`-derived `timeseries.partition_column` trips
/// the unrelated NOT-NULL diagnostic `execute_project`'s pre-execution gate
/// enforces, regardless of locality admission; `KeyDetermined`'s own
/// `pdate` scalar-wrapper projection is likewise not a real GROUP BY key
/// nor an allowlisted aggregate, so `classify_cumulative`'s runtime
/// grammar refuses it independently of locality admission). The
/// conformance gate therefore drives these two routes' actual merge
/// mechanics through
/// `smelt_runtime::maintenance_driver::run_windowed_keyed_maintenance`
/// directly against a real `DuckDbBackend` — the same workaround
/// `crates/smelt-runtime/tests/locality_route3_recurrence_check.rs`
/// already uses for route 3 — rather than through `execute_project`.
#[derive(Debug, Clone)]
pub struct ComposedKeyedRecipe {
    pub model_name: String,
    pub source: SourceRecipe,
    pub route: ComposedRoute,
}

impl ComposedKeyedRecipe {
    pub fn new(route: ComposedRoute) -> Self {
        let source = match route {
            ComposedRoute::KeyEmbedded | ComposedRoute::KeyDetermined => {
                SourceRecipe::events(KeyShape::Single)
            }
            ComposedRoute::RecurrenceBounded => SourceRecipe::events_with_key_recurrence(
                vec!["id".to_string()],
                ROUTE3_RECURRENCE_WINDOW,
            ),
        };
        Self {
            model_name: format!("recipe_composed_{}", composed_route_name(route)),
            source,
            route,
        }
    }

    /// The model's declared `unique_key` (the GROUP BY columns of its own
    /// outermost SELECT — matches `derive_group_by_unique_key`'s
    /// derivation).
    pub fn unique_key(&self) -> Vec<String> {
        match self.route {
            ComposedRoute::KeyEmbedded => vec![
                self.source.key_column.clone(),
                self.source.clock_column.clone(),
            ],
            ComposedRoute::KeyDetermined | ComposedRoute::RecurrenceBounded => {
                vec![self.source.key_column.clone()]
            }
        }
    }

    /// The model's declared `timeseries.partition_column`.
    pub fn partition_column(&self) -> String {
        match self.route {
            ComposedRoute::KeyEmbedded => self.source.clock_column.clone(),
            ComposedRoute::KeyDetermined => "pdate".to_string(),
            ComposedRoute::RecurrenceBounded => "last_seen".to_string(),
        }
    }

    /// The declared `functional_dependencies:` entry (`key`, `determines`)
    /// route 2 needs to admit — `None` for the other two routes.
    pub fn functional_dependency(&self) -> Option<(Vec<String>, String)> {
        match self.route {
            ComposedRoute::KeyDetermined => Some((
                vec![self.source.key_column.clone()],
                self.partition_column(),
            )),
            _ => None,
        }
    }

    /// The coverage-matrix cell id this recipe inhabits (mirrors
    /// [`BodyConstruct::matrix_cell_ids`]'s convention).
    pub fn matrix_cell_id(&self) -> String {
        format!(
            "composed_keyed_{}×append_only",
            composed_route_name(self.route)
        )
    }
}

/// One window of a generated route-3 schedule: `run_date` is the single-day
/// window being driven (always processed in ascending order — the
/// windowed-keyed-maintenance driver's own contract), `rows` are the source
/// rows inserted before that window runs. A storm-key row's own `d` need
/// not equal `run_date` — that mismatch is exactly the "out-of-order
/// redelivery" hazard this generator is scoped to cover (`incremental_models.md`
/// §"Key temporal locality", route 3 "Row movement").
#[derive(Debug, Clone)]
pub struct ComposedRoute3Window {
    pub run_date: chrono::NaiveDate,
    pub rows: Vec<crate::schedule_gen::GenRow>,
}

/// A generated sequence of [`ComposedRoute3Window`]s.
#[derive(Debug, Clone)]
pub struct ComposedRoute3Schedule(pub Vec<ComposedRoute3Window>);

/// Route-3 schedule generator: a fixed 3 disjoint one-day windows, run in
/// ascending order (`run_date = base, base+1, base+2`); every window
/// redelivers [`ROUTE3_STORM_KEY_ID`] with an event-time offset drawn
/// independently per window from `{0, 1}` days off the fixed base date —
/// decoupled from `run_date` (the driver's per-window delta is built
/// directly from each window's own row list — `gate.rs`'s
/// `composed_delta_values_sql` — never filtered off a physical table by
/// `d`, so a window may legitimately redeliver an event-time value
/// *earlier* than a prior window's own event-time: the "out-of-order
/// redelivery, order-independent" adversarial case). The maximum pairwise
/// spread between any two offsets (≤1) and the maximum run-date span
/// (`run_date_max - offset_min` ≤ `2 - 0 = 2`) both stay strictly inside
/// the declared `r = 3` days (`ROUTE3_RECURRENCE_WINDOW`), so every
/// generated case stays **in-bound** by construction. Each window
/// additionally contributes one fresh, never-repeated key — variety
/// alongside the storm. Out-of-bound violation coverage is the dedicated
/// hand-built probe in
/// `crates/smelt-runtime/tests/locality_route3_recurrence_check.rs`, not
/// this pool's job.
pub fn arb_composed_route3_schedule() -> impl Strategy<Value = ComposedRoute3Schedule> {
    const N_WINDOWS: usize = 3;
    let base = chrono::NaiveDate::from_ymd_opt(2024, 3, 1).expect("valid base date");
    proptest::collection::vec(0..=1_i64, N_WINDOWS).prop_map(move |storm_offsets| {
        let mut windows = Vec::new();
        for (i, offset) in storm_offsets.iter().enumerate() {
            let run_date = base + chrono::Duration::days(i as i64);
            let fresh_id = 5_000_i64 + i as i64;
            let rows = vec![
                crate::schedule_gen::GenRow {
                    d: base + chrono::Duration::days(*offset),
                    id: ROUTE3_STORM_KEY_ID,
                    val: 10 + i as i64,
                },
                crate::schedule_gen::GenRow {
                    d: run_date,
                    id: fresh_id,
                    val: 1 + i as i64,
                },
            ];
            windows.push(ComposedRoute3Window { run_date, rows });
        }
        ComposedRoute3Schedule(windows)
    })
}

// =============================================================================
// `EnrichmentEdgeRecipe` (`docs/plans/20260715-composed-axes-conditional-
// maintenance.md` Phase E4): a model-edge enrichment shape for the
// delta-restricted-vs-widened-scan equivalence gate. Styled after the real
// `examples/web_analytics` shape `crates/smelt-runtime/tests/
// web_analytics_session_delta_restriction.rs` already exercises
// (`silver.events_deduped` -> `silver.sessions`, event-grain enrichment):
// column/table names match that fixture so the recipe's own P1 closure
// verdict is derived through the SAME real production entry point
// (`smelt_logical::maintenance::derive::append_model_edge_cells`), not a
// hand-typed classification.
// =============================================================================

/// How the enrichment scope's own join is shaped — exactly one of the four
/// is closure-admissible for a MODEL EDGE (unlike a source edge, a model
/// edge's row-preservation conjunct never has a `referential_integrity`
/// declaration to consult — `derive::model_edge_enrichment_closure` always
/// passes `None` — so only [`EnrichmentJoinKind::LeftJoin`] proves P1
/// `Closed`; both `InnerJoin` and `MembershipPredicate` are closure-failing
/// siblings for two different conjuncts (row preservation, membership
/// predicate)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnrichmentJoinKind {
    /// `LEFT JOIN`, payload-only, no membership predicate — all five
    /// conjuncts prove; P1 is `Closed`.
    LeftJoin,
    /// Bare inner `JOIN` — row preservation (conjunct 4) cannot be proven
    /// for a model edge (no `referential_integrity` world-fact applies);
    /// `Open`.
    InnerJoin,
    /// `LEFT JOIN` plus a `WHERE` predicate testing an enrichment-side
    /// column — membership predicate (conjunct 5) fails; `Open`.
    MembershipPredicate,
}

/// A model-edge enrichment recipe: `silver.events_deduped` (driving,
/// clocked) enriched by a `LEFT JOIN`/`JOIN` against `silver.sessions`
/// (the joined model edge), varied only by [`EnrichmentJoinKind`] — the
/// pool's generative surface for this phase is the SCHEDULE (which keys
/// change and how), not the body shape, mirroring [`crate::dag`]'s own
/// documented convention.
#[derive(Debug, Clone, Copy)]
pub struct EnrichmentEdgeRecipe {
    pub join_kind: EnrichmentJoinKind,
}

impl EnrichmentEdgeRecipe {
    pub fn new(join_kind: EnrichmentJoinKind) -> Self {
        Self { join_kind }
    }

    /// Whether this shape is expected to prove P1 `Closed` — recorded on
    /// the recipe itself (not re-derived by the gate) so the pool's
    /// admission-rate floor can assert against a known expectation.
    pub fn expects_closed(self) -> bool {
        matches!(self.join_kind, EnrichmentJoinKind::LeftJoin)
    }

    /// The driving model edge's bare address — the `Trigger::NewData`
    /// source name this recipe's creation cell keys off of.
    pub fn driving_source(self) -> &'static str {
        "silver.events_deduped"
    }

    /// The joined (enrichment) model edge's bare address.
    pub fn joined_source(self) -> &'static str {
        "silver.sessions"
    }

    /// The two model edges this recipe's scope reads, in the shape
    /// `smelt_logical::maintenance::derive::append_model_edge_cells` expects.
    pub fn model_edges(self) -> Vec<smelt_logical::maintenance::derive::ModelEdge> {
        vec![
            smelt_logical::maintenance::derive::ModelEdge {
                name: self.driving_source().to_string(),
                clock_col: Some("event_date".to_string()),
                unique_key: vec!["event_id".to_string()],
            },
            smelt_logical::maintenance::derive::ModelEdge {
                name: self.joined_source().to_string(),
                clock_col: Some("event_date".to_string()),
                unique_key: vec!["device_id".to_string()],
            },
        ]
    }

    /// The model's own `SELECT` body — column names match
    /// `web_analytics_session_delta_restriction.rs`'s
    /// `EVENTS_ENRICHED_SQL` fixture.
    pub fn model_body(self) -> String {
        let join = match self.join_kind {
            EnrichmentJoinKind::LeftJoin | EnrichmentJoinKind::MembershipPredicate => "LEFT JOIN",
            EnrichmentJoinKind::InnerJoin => "JOIN",
        };
        let where_clause = match self.join_kind {
            EnrichmentJoinKind::MembershipPredicate => " WHERE s.session_utm_campaign IS NOT NULL",
            _ => "",
        };
        format!(
            "SELECT e.event_id, e.device_id, e.event_date, e.utm_campaign AS event_utm_campaign, \
             s.session_id, s.utm_campaign AS session_utm_campaign \
             FROM smelt.silver.events_deduped e {join} smelt.silver.sessions s \
             ON e.device_id = s.device_id{where_clause}"
        )
    }
}

/// A generated schedule for `delta_restricted_equals_widened_scan_at_fixed_s`:
/// [`EnrichmentEdgeRecipe`]'s pool is fixed-shape (three [`EnrichmentJoinKind`]
/// variants); the generative surface is which of `total` fixed baseline keys
/// this run's upstream delta touched (a non-empty, proper subset — at least
/// one key changes, at least one stays untouched so the equivalence claim is
/// non-vacuous).
#[derive(Debug, Clone)]
pub struct EnrichmentEdgeSchedule {
    pub touched_indices: Vec<usize>,
}

pub fn arb_enrichment_edge_recipe() -> impl Strategy<Value = EnrichmentEdgeRecipe> {
    prop_oneof![
        Just(EnrichmentJoinKind::LeftJoin),
        Just(EnrichmentJoinKind::InnerJoin),
        Just(EnrichmentJoinKind::MembershipPredicate),
    ]
    .prop_map(EnrichmentEdgeRecipe::new)
}

pub fn arb_enrichment_edge_schedule(total: usize) -> impl Strategy<Value = EnrichmentEdgeSchedule> {
    proptest::sample::subsequence((0..total).collect::<Vec<usize>>(), 1..total)
        .prop_map(|touched_indices| EnrichmentEdgeSchedule { touched_indices })
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

    /// `ordering_keys_are_unique_by_construction` (plan Phase 5 TDD list):
    /// generator discipline for order-monotone combiners — a sample of
    /// generated ordering-key vectors of every sampled length is always
    /// pairwise distinct, so the documented ties carve-out
    /// (`incremental_models.md` §"Ordering ties") can never fire spuriously
    /// against generated data.
    #[test]
    fn ordering_keys_are_unique_by_construction() {
        let mut runner = TestRunner::deterministic();
        for n in 1..=8_usize {
            let strat = arb_unique_ordering_keys(n);
            for _ in 0..20 {
                let keys = strat.new_tree(&mut runner).unwrap().current();
                assert_eq!(keys.len(), n, "generator must produce exactly n keys");
                let unique: HashSet<i64> = keys.iter().copied().collect();
                assert_eq!(
                    unique.len(),
                    keys.len(),
                    "ordering keys must be pairwise distinct by construction, got {keys:?}"
                );
            }
        }
    }

    /// `keyed_pool_recipes_render_both_combiner_families` — a basic sanity
    /// check that [`KeyedRecipe::new_window_forward`] produces a distinct
    /// model per [`KeyedCombiner`] and its body names the expected
    /// aggregate.
    #[test]
    fn keyed_pool_recipes_render_both_combiner_families() {
        let additive = KeyedRecipe::new_window_forward(KeyedCombiner::Additive);
        let idempotent = KeyedRecipe::new_window_forward(KeyedCombiner::Idempotent);
        assert_ne!(additive.model_name, idempotent.model_name);

        let additive_body = render::render_keyed_model_body(&additive);
        let idempotent_body = render::render_keyed_model_body(&idempotent);
        assert!(additive_body.contains("SUM("));
        assert!(idempotent_body.contains("MAX("));
    }
}
