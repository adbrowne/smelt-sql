use super::builtins::{ALIAS_MAP, REGISTRY};
use super::meta::META_REGISTRY;
use super::*;

/// Canonical registry of SQL built-in signatures (§16 #14/#15, Phase 8).
///
/// Phase 8 seeds the registry with ~30 of the most-commonly-used SQL
/// functions, spanning monomorphic, generic, and variadic shapes. The
/// registry is populated once via [`std::sync::LazyLock`] and stays
/// `'static`; [`BuiltinRegistry::resolve`] folds ASCII case at the
/// lookup boundary.
///
/// Known omissions (documented per Phase 8's scope):
/// * `IS NULL` / `IS NOT NULL` — unary predicates with dedicated SQL
///   syntax, not callable via the function-registry surface. A future
///   rewire may route them through a separate predicate resolver.
/// * `CAST(x AS T)` — also has dedicated SQL syntax; tracked separately
///   from the function registry.
///
/// The registry remains *data only* in Phase 8: inference is still
/// driven by the hand-written match in `smelt-db::type_inference`.
/// Phase 9 rewires `infer_function_type` through this registry.
pub struct BuiltinRegistry;

impl BuiltinRegistry {
    /// Resolve a built-in by name, case-insensitively (ASCII folding).
    ///
    /// Checks the canonical name table first, then dialect aliases (e.g.
    /// `NVL` → `IFNULL`'s entry, `GET_JSON_OBJECT` → `JSON_EXTRACT_TEXT`'s
    /// entry). Returns `Some(&'static Signature)` when the name matches a
    /// registered entry or a registered alias of one, `None` otherwise.
    pub fn resolve(name: &str) -> Option<&'static Signature> {
        let upper = name.to_ascii_uppercase();
        if let Some(sig) = REGISTRY.get(&upper) {
            return Some(sig);
        }
        let canonical = ALIAS_MAP.get(&upper)?;
        REGISTRY.get(canonical)
    }

    /// Resolve a name (canonical or dialect alias) to its canonical
    /// (upper-cased) registry name, case-insensitively.
    ///
    /// This is the single alias-resolution entry point other crates use to
    /// map a dialect spelling back to the name `SqlFunction` recognises —
    /// keeping alias recognition registry-owned per architecture.md
    /// §Constraints #14.
    pub fn canonical_name(name: &str) -> Option<&'static str> {
        Self::resolve(name).map(|sig| sig.name.as_str())
    }

    /// Iterator over all canonical (upper-cased) names in the registry.
    pub fn names() -> impl Iterator<Item = &'static str> {
        REGISTRY.keys().map(|s| s.as_str())
    }

    /// Iterator over every registered `(alias, canonical_name)` pair,
    /// upper-cased. Used by the registry-consistency gate to assert every
    /// alias is recognized and classified consistently with its canonical
    /// entry.
    pub fn aliases() -> impl Iterator<Item = (&'static str, &'static str)> {
        ALIAS_MAP
            .iter()
            .map(|(alias, canonical)| (alias.as_str(), canonical.as_str()))
    }

    /// Look up a smelt meta-builtin by its dotted path name (case-insensitive).
    ///
    /// These are smelt-specific meta-language builtins that operate on meta types
    /// (`SmeltType`) rather than SQL types. Examples: `smelt.columns_of`.
    ///
    /// Returns `Some(&'static SmeltMetaSignature)` when the name matches, `None`
    /// otherwise.
    pub fn lookup(name: &str) -> Option<&'static SmeltMetaSignature> {
        META_REGISTRY.get(&name.to_ascii_lowercase())
    }
}
