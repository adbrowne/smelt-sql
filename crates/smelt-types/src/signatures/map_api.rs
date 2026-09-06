use super::*;
use std::collections::BTreeMap;

// ============================================================================
// Map API registry (Phase E1)
// ============================================================================

/// Arity descriptor for Map API methods and future built-in registries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arity {
    /// Exactly `n` positional arguments required (no variadic).
    Exact(usize),
}

/// Discriminates the dispatch behaviour of a Map API method.
///
/// Adding a new method to the registry requires choosing a `kind`, which
/// determines whether key-type validation and static-key resolution are
/// performed at the call site. This makes the registry the sole source of
/// truth: changing a method's kind changes its dispatch behaviour without
/// touching call-site code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapApiMethodKind {
    /// Zero-argument iteration method (`entries`, `keys`, `values`).
    /// No key argument — no key-type check, no static-key resolution.
    ZeroArg,
    /// One-argument lookup that resolves to the value type (`get`).
    /// Validates the key-argument type; resolves statically to the per-entry
    /// type when the key is a string literal and the map contents are known.
    KeyedGet,
    /// One-argument presence check that resolves to `Boolean` (`has`).
    /// Validates the key-argument type; resolves statically to `Bool(true)`
    /// or `Bool(false)` when the key is a string literal and map contents
    /// are known.
    KeyedHas,
}

/// A single entry in the closed Map API method registry.
///
/// The five entries are: `entries`, `keys`, `values`, `get`, `has`.
/// Named arguments are never supported on Map API methods.
pub struct MapApiMethod {
    /// The method name (e.g. `"entries"`).
    pub name: &'static str,
    /// Required positional argument count.
    pub arity: Arity,
    /// Dispatch kind — controls key-arg validation and static-resolution behaviour.
    pub kind: MapApiMethodKind,
    /// Whether named arguments are accepted (always `false` in v1).
    pub named_args_allowed: bool,
    /// Return type formula. Takes the receiver's `K` and `V` types and
    /// returns the synthesised result type. The formula uses owned values
    /// so the returned `SmeltType` is self-contained.
    pub return_type_formula: fn(&SmeltType, &SmeltType) -> SmeltType,
}

impl std::fmt::Debug for MapApiMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "MapApiMethod {{ name: {:?}, arity: {:?}, kind: {:?} }}",
            self.name, self.arity, self.kind
        )
    }
}

/// Build the `List<{key: K, value: V}>` return type for `m.entries()`.
fn map_entries_return(k: &SmeltType, v: &SmeltType) -> SmeltType {
    let mut fields = BTreeMap::new();
    fields.insert("key".to_string(), k.clone());
    fields.insert("value".to_string(), v.clone());
    SmeltType::List(Box::new(SmeltType::Record { fields, name: None }))
}

/// Build `List<K>` for `m.keys()`.
fn map_keys_return(k: &SmeltType, _v: &SmeltType) -> SmeltType {
    SmeltType::List(Box::new(k.clone()))
}

/// Build `List<V>` for `m.values()`.
fn map_values_return(_k: &SmeltType, v: &SmeltType) -> SmeltType {
    SmeltType::List(Box::new(v.clone()))
}

/// Build `V` for `m.get(k)`.
fn map_get_return(_k: &SmeltType, v: &SmeltType) -> SmeltType {
    v.clone()
}

/// Build `Boolean` for `m.has(k)`.
fn map_has_return(_k: &SmeltType, _v: &SmeltType) -> SmeltType {
    SmeltType::Expr(TypeConstraint::Concrete(crate::DataType::Boolean))
}

/// Closed Map API method registry (Phase E1).
///
/// The five entries are the entire Map surface in v1.
/// `entries`, `keys`, `values` — arity 0 (no arguments).
/// `get`, `has` — arity 1 (one positional key argument).
///
/// Named arguments are not permitted on any Map API method.
pub static MAP_API_METHODS: &[MapApiMethod] = &[
    MapApiMethod {
        name: "entries",
        arity: Arity::Exact(0),
        kind: MapApiMethodKind::ZeroArg,
        named_args_allowed: false,
        return_type_formula: map_entries_return,
    },
    MapApiMethod {
        name: "keys",
        arity: Arity::Exact(0),
        kind: MapApiMethodKind::ZeroArg,
        named_args_allowed: false,
        return_type_formula: map_keys_return,
    },
    MapApiMethod {
        name: "values",
        arity: Arity::Exact(0),
        kind: MapApiMethodKind::ZeroArg,
        named_args_allowed: false,
        return_type_formula: map_values_return,
    },
    MapApiMethod {
        name: "get",
        arity: Arity::Exact(1),
        kind: MapApiMethodKind::KeyedGet,
        named_args_allowed: false,
        return_type_formula: map_get_return,
    },
    MapApiMethod {
        name: "has",
        arity: Arity::Exact(1),
        kind: MapApiMethodKind::KeyedHas,
        named_args_allowed: false,
        return_type_formula: map_has_return,
    },
];

/// Look up a Map API method by name. Returns `None` for any name outside
/// the closed set `{entries, keys, values, get, has}`.
pub fn lookup_map_api_method(name: &str) -> Option<&'static MapApiMethod> {
    MAP_API_METHODS.iter().find(|m| m.name == name)
}
