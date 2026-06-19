//! CLI argument resolution: scope computation and model-name resolution.
//!
//! Implements `docs/specs/cli.md` §"Argument resolution and `--scope`" and
//! §"Cwd-derived scope computation" as pure functions consumed by every CLI
//! command that accepts an entity identifier.

use std::path::Path;

/// An active scope: a dot-separated prefix that narrows argument resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    /// The individual path segments, e.g. `["silver"]` or `["marts", "daily"]`.
    pub segments: Vec<String>,
}

impl Scope {
    /// Build a `Scope` from a dot-separated string, e.g. `"silver"` or
    /// `"marts.daily"`.
    pub fn from_dot_path(s: &str) -> Self {
        Scope {
            segments: s.split('.').map(|seg| seg.to_string()).collect(),
        }
    }

    /// Return the dot-joined prefix string, e.g. `"silver"` or `"marts.daily"`.
    pub fn as_prefix(&self) -> String {
        self.segments.join(".")
    }
}

/// Validate a user-supplied `--scope` value.  Returns `Err` with a
/// human-readable message if the value is rejected.
pub fn validate_scope_value(value: &str) -> Result<(), String> {
    // Reject leading `smelt.` (the user should pass the path without the
    // top-level smelt namespace).
    if value.starts_with("smelt.") || value == "smelt" {
        return Err(format!(
            "--scope value '{}' must not start with 'smelt.' — pass the path below the smelt namespace",
            value
        ));
    }
    // Reject values that have leading/trailing whitespace.
    if value != value.trim() {
        return Err(format!(
            "--scope value '{}' must not contain leading or trailing whitespace",
            value
        ));
    }
    Ok(())
}

/// Compute the active scope from explicit flag or cwd-derivation.
///
/// Precedence (highest to lowest):
/// 1. `explicit = Some(non-empty)` — use that value directly.
/// 2. `explicit = Some("")` — force no scope (override cwd derivation).
/// 3. `explicit = None` — derive from `cwd` relative to `project_root` and
///    `scan_roots`.
///
/// `scan_roots` are the entries from `paths:` in `smelt.yml`, each relative to
/// `project_root`.
pub fn compute_scope(
    project_root: &Path,
    cwd: &Path,
    scan_roots: &[String],
    explicit: Option<&str>,
) -> Option<Scope> {
    match explicit {
        // Explicit non-empty scope takes priority.
        Some(s) if !s.is_empty() => Some(Scope::from_dot_path(s)),
        // Empty string forces no scope.
        Some(_) => None,
        // No explicit flag — derive from cwd.
        None => derive_scope_from_cwd(project_root, cwd, scan_roots),
    }
}

/// Derive an auto-scope from the current working directory.
///
/// Algorithm (from `docs/specs/cli.md` §"Cwd-derived scope computation"):
/// 1. Strip `project_root` prefix from `cwd`. If cwd is not under project root,
///    no auto-scope.
/// 2. For each `scan_root` in `paths:` (declaration order), check whether `rel`
///    starts with that scan root. First match wins.
/// 3. Match → auto-scope is the remaining path components, joined by `.`.
///    Empty result (cwd IS the scan root) → no scope.
/// 4. No match → no scope.
fn derive_scope_from_cwd(project_root: &Path, cwd: &Path, scan_roots: &[String]) -> Option<Scope> {
    // Step 1: strip project root
    let rel = cwd.strip_prefix(project_root).ok()?;

    // Step 2: try each scan root
    for scan_root_str in scan_roots {
        let scan_root = Path::new(scan_root_str);
        if let Ok(below_scan_root) = rel.strip_prefix(scan_root) {
            // Step 3: collect the remaining components
            let components: Vec<String> = below_scan_root
                .components()
                .filter_map(|c| {
                    if let std::path::Component::Normal(os) = c {
                        os.to_str().map(|s| s.to_string())
                    } else {
                        None
                    }
                })
                .collect();

            if components.is_empty() {
                // cwd is the scan root itself — no scope
                return None;
            }

            return Some(Scope {
                segments: components,
            });
        }
    }

    // Step 4: no scan root matched
    None
}

/// Error type for entity resolution failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolutionError {
    /// No entity resolved for `arg`; `hints` lists nearby entities (e.g.
    /// `did you mean 'silver.events_parsed'?`).
    NotFound { arg: String, hints: Vec<String> },
    /// No scope was set and `arg` is a bare leaf matching multiple entities.
    Ambiguous { arg: String, matches: Vec<String> },
}

impl std::fmt::Display for ResolutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolutionError::NotFound { arg, hints } => {
                write!(f, "Model '{}' not found", arg)?;
                if hints.len() == 1 {
                    write!(f, ". Did you mean '{}'?", hints[0])?;
                } else if !hints.is_empty() {
                    write!(f, ". Candidates: {}", hints.join(", "))?;
                }
                Ok(())
            }
            ResolutionError::Ambiguous { arg, matches } => {
                write!(
                    f,
                    "Ambiguous model name '{}': matches {}",
                    arg,
                    matches.join(", ")
                )
            }
        }
    }
}

/// Strip the `smelt.` namespace prefix from a canonical path returned by
/// `smelt_db::leaf_did_you_mean` or `smelt_db::resolve_ref_path`, producing
/// the CLI-layer canonical form (e.g. `"silver.events_parsed"` from
/// `"smelt.silver.events_parsed"`).
fn strip_smelt_prefix(path: &str) -> &str {
    path.strip_prefix("smelt.").unwrap_or(path)
}

/// Resolve a CLI argument to a canonical path string.
///
/// Implements `docs/specs/cli.md` §"Argument resolution algorithm":
/// 1. Determine the active scope.
/// 2. Build candidates: if scope is `Some(s)`, candidates are
///    `[s ++ arg_segs, arg_segs]`; if `None`, only `arg_segs`.
/// 3. Resolve each candidate against the workspace via
///    [`smelt_db::resolve_ref_path`]. First match wins.
/// 4. No candidate resolves → "not found" diagnostic with hints via
///    [`smelt_db::leaf_did_you_mean`].
/// 5. No scope + bare leaf + multiple entities have that leaf → "ambiguous".
///
/// Uses [`smelt_db::leaf_did_you_mean`] for hint computation so that the
/// same logic is shared with the LSP's `UndefinedModelRef` diagnostic.
pub fn resolve_argument(
    db: &smelt_db::Database,
    workspace: smelt_db::Workspace,
    project: smelt_db::ProjectInput,
    scope: Option<&Scope>,
    arg: &str,
) -> Result<String, ResolutionError> {
    // Step 0: strip leading `smelt.` prefix so the canonical printed form
    // (`smelt.silver.events_parsed`) round-trips back into the same entity
    // as the bare form (`silver.events_parsed`).
    let arg = strip_smelt_prefix(arg);
    let arg_segs: Vec<&str> = arg.split('.').collect();

    // Build candidate segment lists
    let candidates: Vec<Vec<String>> = if let Some(s) = scope {
        let scoped: Vec<String> = s
            .segments
            .iter()
            .map(|seg| seg.as_str())
            .chain(arg_segs.iter().copied())
            .map(|seg| seg.to_string())
            .collect();
        vec![scoped, arg_segs.iter().map(|s| s.to_string()).collect()]
    } else {
        vec![arg_segs.iter().map(|s| s.to_string()).collect()]
    };

    // Resolve each candidate — first hit wins
    for candidate in &candidates {
        if smelt_db::resolve_ref_path(db, workspace, candidate.clone()).is_some() {
            return Ok(candidate.join("."));
        }
    }

    // No candidate resolved. Compute hints via smelt_db::leaf_did_you_mean.
    let leaf = arg_segs.last().copied().unwrap_or(arg);

    // leaf_did_you_mean returns "smelt."-prefixed canonical paths; strip prefix
    // for the CLI layer.
    let leaf_matches: Vec<String> = smelt_db::leaf_did_you_mean(db, workspace, Some(project), leaf)
        .into_iter()
        .map(|p| strip_smelt_prefix(&p).to_string())
        .collect();

    // No-scope bare leaf with multiple matches → Ambiguous
    if scope.is_none() && arg_segs.len() == 1 && leaf_matches.len() > 1 {
        return Err(ResolutionError::Ambiguous {
            arg: arg.to_string(),
            matches: leaf_matches,
        });
    }

    Err(ResolutionError::NotFound {
        arg: arg.to_string(),
        hints: leaf_matches,
    })
}

/// Resolve a list of selector strings (e.g. from `--select`) through scope
/// expansion, returning the resolved canonical forms.
///
/// Selectors that contain `:` (e.g. `tag:revenue`, `generator_file:...`) pass
/// through unchanged — they are not model-name references.  All other
/// selectors are treated as model-name arguments and resolved via
/// [`resolve_argument`].  Leading and trailing `+` (upstream/downstream
/// expansion modifiers) are preserved around the resolved name.
///
/// Returns `Err` with a human-readable message on the first failure.
pub fn resolve_selector_args(
    db: &smelt_db::Database,
    workspace: smelt_db::Workspace,
    project: smelt_db::ProjectInput,
    scope: Option<&Scope>,
    selectors: &[String],
) -> Result<Vec<String>, ResolutionError> {
    let mut resolved = Vec::with_capacity(selectors.len());
    for sel in selectors {
        // Strip leading `+` and trailing `+` modifiers.
        let leading_plus = sel.starts_with('+');
        let s = if leading_plus {
            &sel[1..]
        } else {
            sel.as_str()
        };
        let trailing_plus = s.ends_with('+');
        let inner = if trailing_plus { &s[..s.len() - 1] } else { s };

        if inner.contains(':') {
            // Tag / generator-file / other colon-form selector — pass through unchanged.
            resolved.push(sel.clone());
        } else {
            // Model-name selector — apply scope resolution.
            let canonical = resolve_argument(db, workspace, project, scope, inner)?;
            let prefix = if leading_plus { "+" } else { "" };
            let suffix = if trailing_plus { "+" } else { "" };
            resolved.push(format!("{}{}{}", prefix, canonical, suffix));
        }
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scope(s: &str) -> Scope {
        Scope::from_dot_path(s)
    }

    /// Build a Salsa database with the given (path, content) pairs as SQL model
    /// files, all under `project_root`.  No `smelt.yml` on disk — scan roots
    /// default to empty, so `file_path_tuple` does NOT strip any directory
    /// prefix and paths read as `<dir>/<leaf>`.
    fn build_db(
        project_root: PathBuf,
        files: &[(PathBuf, &str)],
    ) -> (
        smelt_db::Database,
        smelt_db::Workspace,
        smelt_db::ProjectInput,
    ) {
        use smelt_db::Database;
        let mut db = Database::default();
        let project = db.set_project_input(project_root.clone(), String::new());
        let mut handles = Vec::with_capacity(files.len());
        for (path, content) in files {
            let sf = db.set_source_file(path.clone(), (*content).to_string(), project_root.clone());
            handles.push(sf);
        }
        db.set_workspace(handles, vec![project]);
        let ws = db.workspace();
        (db, ws, project)
    }

    // ── compute_scope tests ────────────────────────────────────────────────

    #[test]
    fn auto_scope_from_cwd_under_scan_root() {
        // cwd = <project>/models/silver → scope = Some(["silver"])
        let project = PathBuf::from("/project");
        let cwd = PathBuf::from("/project/models/silver");
        let scan_roots = vec!["models".to_string()];
        let result = compute_scope(&project, &cwd, &scan_roots, None);
        assert_eq!(result, Some(scope("silver")));
    }

    #[test]
    fn auto_scope_from_cwd_deep_under_scan_root() {
        // cwd = <project>/models/marts/daily → scope = Some(["marts", "daily"])
        let project = PathBuf::from("/project");
        let cwd = PathBuf::from("/project/models/marts/daily");
        let scan_roots = vec!["models".to_string()];
        let result = compute_scope(&project, &cwd, &scan_roots, None);
        assert_eq!(
            result,
            Some(Scope {
                segments: vec!["marts".to_string(), "daily".to_string()]
            })
        );
    }

    #[test]
    fn auto_scope_from_cwd_at_scan_root() {
        // cwd = <project>/models → scope = None (cwd IS the scan root)
        let project = PathBuf::from("/project");
        let cwd = PathBuf::from("/project/models");
        let scan_roots = vec!["models".to_string()];
        let result = compute_scope(&project, &cwd, &scan_roots, None);
        assert_eq!(result, None);
    }

    #[test]
    fn auto_scope_from_cwd_at_project_root() {
        // cwd = <project> → scope = None (no scan root matched at root level)
        let project = PathBuf::from("/project");
        let cwd = PathBuf::from("/project");
        let scan_roots = vec!["models".to_string()];
        let result = compute_scope(&project, &cwd, &scan_roots, None);
        assert_eq!(result, None);
    }

    #[test]
    fn auto_scope_from_cwd_outside_project() {
        // cwd = /tmp → scope = None (not under project root)
        let project = PathBuf::from("/project");
        let cwd = PathBuf::from("/tmp");
        let scan_roots = vec!["models".to_string()];
        let result = compute_scope(&project, &cwd, &scan_roots, None);
        assert_eq!(result, None);
    }

    #[test]
    fn explicit_scope_overrides_cwd() {
        // --scope marts with cwd inside models/silver → scope = Some(["marts"])
        let project = PathBuf::from("/project");
        let cwd = PathBuf::from("/project/models/silver");
        let scan_roots = vec!["models".to_string()];
        let result = compute_scope(&project, &cwd, &scan_roots, Some("marts"));
        assert_eq!(result, Some(scope("marts")));
    }

    #[test]
    fn empty_scope_disables_auto() {
        // --scope "" with cwd inside models/silver → scope = None
        let project = PathBuf::from("/project");
        let cwd = PathBuf::from("/project/models/silver");
        let scan_roots = vec!["models".to_string()];
        let result = compute_scope(&project, &cwd, &scan_roots, Some(""));
        assert_eq!(result, None);
    }

    #[test]
    fn scope_rejects_leading_smelt() {
        assert!(validate_scope_value("smelt.silver").is_err());
        assert!(validate_scope_value("smelt").is_err());
    }

    #[test]
    fn scope_rejects_whitespace() {
        assert!(validate_scope_value(" silver ").is_err());
        assert!(validate_scope_value("silver ").is_err());
        assert!(validate_scope_value(" silver").is_err());
    }

    #[test]
    fn scope_accepts_valid() {
        assert!(validate_scope_value("silver").is_ok());
        assert!(validate_scope_value("marts.daily").is_ok());
        assert!(validate_scope_value("").is_ok()); // empty is allowed (means no scope)
    }

    // ── resolve_argument tests ─────────────────────────────────────────────
    //
    // Note: no smelt.yml → scan roots are empty → file paths are NOT stripped.
    // A file at `<root>/silver/events_parsed.sql` resolves to `silver.events_parsed`.

    #[test]
    fn resolve_arg_scoped_match_first() {
        // workspace has silver.events_parsed; with scope silver, events_parsed → silver.events_parsed
        let root = PathBuf::from("/test/scoped_match");
        let (db, ws, proj) = build_db(
            root.clone(),
            &[
                (root.join("bronze/raw_events.sql"), "SELECT 1 AS id\n"),
                (
                    root.join("silver/events_parsed.sql"),
                    "SELECT 1 AS event_id\n",
                ),
            ],
        );
        let s = scope("silver");
        let result = resolve_argument(&db, ws, proj, Some(&s), "events_parsed");
        assert_eq!(result, Ok("silver.events_parsed".to_string()));
    }

    #[test]
    fn resolve_arg_falls_through_to_bare() {
        // workspace has silver.events_parsed AND bronze.events_parsed;
        // scope=gold means gold.bronze.events_parsed doesn't exist,
        // so the bare arg bronze.events_parsed resolves directly.
        let root = PathBuf::from("/test/falls_through");
        let (db, ws, proj) = build_db(
            root.clone(),
            &[
                (
                    root.join("silver/events_parsed.sql"),
                    "SELECT 1 AS event_id\n",
                ),
                (
                    root.join("bronze/events_parsed.sql"),
                    "SELECT 1 AS event_id\n",
                ),
            ],
        );
        let s = scope("gold");
        let result = resolve_argument(&db, ws, proj, Some(&s), "bronze.events_parsed");
        assert_eq!(result, Ok("bronze.events_parsed".to_string()));
    }

    #[test]
    fn resolve_arg_bare_leaf_no_scope_ambiguous_errors() {
        // workspace has silver.events AND bronze.events; no scope, arg=events → Ambiguous
        let root = PathBuf::from("/test/ambiguous");
        let (db, ws, proj) = build_db(
            root.clone(),
            &[
                (root.join("silver/events.sql"), "SELECT 1 AS event_id\n"),
                (root.join("bronze/events.sql"), "SELECT 1 AS event_id\n"),
            ],
        );
        let result = resolve_argument(&db, ws, proj, None, "events");
        match result {
            Err(ResolutionError::Ambiguous { arg, matches }) => {
                assert_eq!(arg, "events");
                assert!(matches.contains(&"silver.events".to_string()));
                assert!(matches.contains(&"bronze.events".to_string()));
            }
            other => panic!("expected Ambiguous, got {:?}", other),
        }
    }

    #[test]
    fn resolve_arg_bare_leaf_no_scope_single_match_hints() {
        // workspace has only silver.events_parsed; no scope, arg=events_parsed
        // → NotFound with did-you-mean hint
        let root = PathBuf::from("/test/single_hint");
        let (db, ws, proj) = build_db(
            root.clone(),
            &[(
                root.join("silver/events_parsed.sql"),
                "SELECT 1 AS event_id\n",
            )],
        );
        let result = resolve_argument(&db, ws, proj, None, "events_parsed");
        match result {
            Err(ResolutionError::NotFound { arg, hints }) => {
                assert_eq!(arg, "events_parsed");
                assert_eq!(hints, vec!["silver.events_parsed"]);
            }
            other => panic!("expected NotFound, got {:?}", other),
        }
    }

    #[test]
    fn resolve_arg_full_path_always_works() {
        // workspace has silver.events_parsed; with scope=marts,
        // arg=silver.events_parsed → first candidate marts.silver.events_parsed fails,
        // second candidate silver.events_parsed succeeds.
        let root = PathBuf::from("/test/full_path");
        let (db, ws, proj) = build_db(
            root.clone(),
            &[(
                root.join("silver/events_parsed.sql"),
                "SELECT 1 AS event_id\n",
            )],
        );
        let s = scope("marts");
        let result = resolve_argument(&db, ws, proj, Some(&s), "silver.events_parsed");
        assert_eq!(result, Ok("silver.events_parsed".to_string()));
    }

    #[test]
    fn resolve_arg_exact_full_path_no_scope() {
        // No scope, arg=silver.events_parsed (full path) → resolves directly
        let root = PathBuf::from("/test/exact_full");
        let (db, ws, proj) = build_db(
            root.clone(),
            &[
                (
                    root.join("silver/events_parsed.sql"),
                    "SELECT 1 AS event_id\n",
                ),
                (root.join("bronze/raw_events.sql"), "SELECT 1 AS id\n"),
            ],
        );
        let result = resolve_argument(&db, ws, proj, None, "silver.events_parsed");
        assert_eq!(result, Ok("silver.events_parsed".to_string()));
    }

    #[test]
    fn resolve_arg_not_found_no_hints() {
        // arg not found and no leaf match anywhere
        let root = PathBuf::from("/test/not_found");
        let (db, ws, proj) = build_db(
            root.clone(),
            &[(
                root.join("silver/events_parsed.sql"),
                "SELECT 1 AS event_id\n",
            )],
        );
        let result = resolve_argument(&db, ws, proj, None, "missing_model");
        match result {
            Err(ResolutionError::NotFound { arg, hints }) => {
                assert_eq!(arg, "missing_model");
                assert!(hints.is_empty());
            }
            other => panic!("expected NotFound, got {:?}", other),
        }
    }

    // ── D-36: canonical smelt. prefix strip tests ───────────────────────────

    #[test]
    fn resolve_arg_strips_smelt_prefix() {
        // smelt.silver.events_parsed and silver.events_parsed resolve to the same entity.
        let root = PathBuf::from("/test/smelt_prefix");
        let (db, ws, proj) = build_db(
            root.clone(),
            &[(
                root.join("silver/events_parsed.sql"),
                "SELECT 1 AS event_id\n",
            )],
        );
        let with_prefix = resolve_argument(&db, ws, proj, None, "smelt.silver.events_parsed");
        let without_prefix = resolve_argument(&db, ws, proj, None, "silver.events_parsed");
        assert_eq!(with_prefix, Ok("silver.events_parsed".to_string()));
        assert_eq!(with_prefix, without_prefix);
    }

    #[test]
    fn resolve_arg_smelt_prefix_stripped_before_scope_expansion() {
        // With scope=silver, smelt.bronze.x should resolve bronze.x (a full path),
        // not silver.smelt.bronze.x or silver.bronze.x.
        let root = PathBuf::from("/test/smelt_prefix_scope");
        let (db, ws, proj) = build_db(
            root.clone(),
            &[
                (root.join("bronze/raw_events.sql"), "SELECT 1 AS id\n"),
                (
                    root.join("silver/events_parsed.sql"),
                    "SELECT 1 AS event_id\n",
                ),
            ],
        );
        let s = scope("silver");
        // smelt.bronze.raw_events → strip prefix → bronze.raw_events (full path, bypasses scope)
        let result = resolve_argument(&db, ws, proj, Some(&s), "smelt.bronze.raw_events");
        assert_eq!(result, Ok("bronze.raw_events".to_string()));
    }

    #[test]
    fn resolve_selector_args_strips_smelt_prefix() {
        // --select smelt.silver.events_parsed resolves the same as silver.events_parsed
        let root = PathBuf::from("/test/smelt_prefix_selector");
        let (db, ws, proj) = build_db(
            root.clone(),
            &[(
                root.join("silver/events_parsed.sql"),
                "SELECT 1 AS event_id\n",
            )],
        );
        let selectors = vec!["smelt.silver.events_parsed".to_string()];
        let result = resolve_selector_args(&db, ws, proj, None, &selectors);
        assert_eq!(result, Ok(vec!["silver.events_parsed".to_string()]));
    }
}
