/// The grain label a contract-lattice validator names in a refusal message
/// — `smelt_core::config::Grain` (the declarable surface) widened with the
/// succession grain, which is never declared, only derived
/// (`analysis::succession::classify_keyed_succession`). Lives here, in
/// `smelt-logical`, rather than `smelt_core::config`, because a succession
/// verdict is not a value a model's frontmatter can ever write
/// (`CLAUDE.md` §Architectural invariants "Contract-lattice point single
/// ownership").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrainLabel {
    Partition,
    Key,
    KeyPerPartition,
    Succession,
}

impl std::fmt::Display for GrainLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            GrainLabel::Partition => "partition",
            GrainLabel::Key => "key",
            GrainLabel::KeyPerPartition => "key_per_partition",
            GrainLabel::Succession => "succession (keyed succession / SCD2)",
        };
        write!(f, "{s}")
    }
}

impl From<smelt_core::config::Grain> for GrainLabel {
    fn from(grain: smelt_core::config::Grain) -> Self {
        match grain {
            smelt_core::config::Grain::Partition => GrainLabel::Partition,
            smelt_core::config::Grain::Key => GrainLabel::Key,
            smelt_core::config::Grain::KeyPerPartition => GrainLabel::KeyPerPartition,
        }
    }
}
