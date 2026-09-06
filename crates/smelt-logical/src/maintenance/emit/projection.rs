//! Select-list rewrites for decomposed (rung 2) state: aggregator-column
//! fold expansion, the staged-candidate select, state-augmented
//! projection, and presentation projection.

use super::types::*;

// ── Decomposed-state column fold expansion (`docs/specs/
// incremental_shapes.md` §"Decomposed state (rung 2) in keyed models",
// "Combiner over state") ─────────────────────────────────────────────────

/// Expand one [`AggregatorColumn`](crate::rules::cumulative::AggregatorColumn)
/// into its `(column, combine_expression)` fold pairs for the `MERGE`'s
/// `SET` clause. Single-owner statement rule (`docs/specs/
/// incremental_models.md` §"Statement emission (single owner)"): both the
/// executed keyed-fold `MERGE` (`smelt-runtime::cumulative::
/// build_cumulative_merge_sql`) and the `smelt explain` preview
/// (`smelt-runtime::diagnostics`) call this so their fold shapes can never
/// diverge (`docs/outcomes/20260809-rung2-state-shapes` row 7).
///
/// A stateless column (`state: None`, every family admitted before this
/// mechanism existed) still produces exactly the one pair it always has. A
/// state-bearing column expands into one pair per hidden state column (each
/// folded by its own combiner over `target.<c>`/`delta.<c>`) plus the
/// presented column, set to the presentation expression with every
/// state-column reference substituted by that column's own *merged*
/// expression — so the presented value is always recomputed fresh from the
/// just-merged state, never folded directly.
pub fn expand_aggregator_column_folds(
    col: &crate::rules::cumulative::AggregatorColumn,
) -> Vec<(String, String)> {
    let Some(state) = &col.state else {
        let target_col = format!("target.{}", col.output_name);
        let delta_col = format!("delta.{}", col.output_name);
        let expr = col.cross_partition_combiner.render(&target_col, &delta_col);
        return vec![(col.output_name.clone(), expr)];
    };

    let mut folds: Vec<(String, String)> = Vec::with_capacity(state.state_columns.len() + 1);
    let mut merged_by_name: Vec<(String, String)> = Vec::with_capacity(state.state_columns.len());
    for state_col in &state.state_columns {
        let target_col = format!("target.{}", state_col.name);
        let delta_col = format!("delta.{}", state_col.name);
        let merged = state_col.combiner.render(&target_col, &delta_col);
        merged_by_name.push((state_col.name.clone(), merged.clone()));
        folds.push((state_col.name.clone(), merged));
    }
    // One simultaneous pass over the ORIGINAL presentation expression, not a
    // chain of dependent substitutions — a state column's own merged
    // expression can embed another state column's qualified name (the
    // order-monotone `v` column's fold text names its sibling `o` column,
    // e.g. `target.status__o`), and re-scanning already-substituted text for
    // the next name would corrupt it (`docs/outcomes/
    // 20260809-rung2-state-shapes` row 5).
    let presentation_expr = substitute_identifiers(&state.presentation_expr, &merged_by_name);
    folds.push((col.output_name.clone(), presentation_expr));
    folds
}

/// Replace every whole-identifier occurrence of each `(name, replacement)`
/// pair in `text`, in one simultaneous left-to-right pass over the
/// ORIGINAL `text` — a match must not be preceded or followed by another
/// identifier character (`[A-Za-z0-9_]`), so `avg_amount__sum` is not
/// matched inside `avg_amount__sum_2`. A single pass (rather than N
/// sequential single-name substitutions) matters: a replacement text can
/// itself contain another pair's `name` as a substring (a state column's
/// merged fold expression naming a sibling state column), and re-scanning
/// already-substituted output for the next name would corrupt it. Used to
/// rewrite a `DecomposedState` presentation expression's state-column
/// references onto their merged fold expressions
/// (`expand_aggregator_column_folds`) — plain string substitution over SQL
/// identifiers, not general SQL rewriting.
fn substitute_identifiers(text: &str, replacements: &[(String, String)]) -> String {
    fn is_ident_char(c: char) -> bool {
        c.is_ascii_alphanumeric() || c == '_'
    }

    let bytes = text.as_bytes();
    let mut result = String::with_capacity(text.len());
    let mut skip_until = 0usize;
    for (i, ch) in text.char_indices() {
        if i < skip_until {
            continue;
        }
        let matched = replacements.iter().find(|(name, _)| {
            text[i..].starts_with(name.as_str())
                && (i == 0 || !is_ident_char(bytes[i - 1] as char))
                && {
                    let after = i + name.len();
                    after >= bytes.len() || !is_ident_char(bytes[after] as char)
                }
        });
        if let Some((name, replacement)) = matched {
            result.push('(');
            result.push_str(replacement);
            result.push(')');
            skip_until = i + name.len();
            continue;
        }
        result.push(ch);
    }
    result
}

/// The post-fold candidate rows for a keyed fold (`docs/outcomes/
/// 20260815-definition-delta-migrate/phases/27d-plan.md`): a keyed fold's
/// staged candidate is `combiner(stored, delta)`, not the raw delta — the
/// merge-less staged-candidate mechanism ([`emit_staged_candidate_conditional`])
/// needs this select as its `candidate_select` to realise a `KeyedFold`
/// cell, mirroring what [`emit_keyed_fold`]'s `MERGE … WHEN MATCHED THEN
/// UPDATE SET` already computes for the `MERGE`-capable path.
///
/// `SELECT <key>, <folds> FROM (<delta_sql>) AS delta LEFT JOIN <table> AS
/// target ON <key join>` — a `LEFT JOIN` of the delta to the target so every
/// delta key is represented even when the target has no stored row for it
/// yet. A matched key (the target row exists) resolves each fold column to
/// `folds`'s own combine expression, exactly like `emit_keyed_fold`'s
/// matched arm. A delta-only key (no stored row — `target`'s join columns
/// are `NULL`) resolves each fold column to its own **raw delta value**
/// instead: `folds`'s combine expressions are written in terms of
/// `target.*`/`delta.*` assuming a matched row (e.g. `target.c + delta.c` for
/// a `Sum` combiner), so applying them unmodified to an absent target would
/// produce `NULL` rather than the delta's own value — the same "insert the
/// delta row as-is" contract `emit_keyed_fold`'s `WHEN NOT MATCHED THEN
/// INSERT` arm already has.
///
/// # Panics
/// Panics if `key` is empty — an identity-free call has no join to build
/// (mirrors [`emit_staged_candidate_conditional`]'s own empty-key panic).
pub fn keyed_fold_candidate_select(
    table: &str,
    key: &[String],
    folds: &[(String, String)],
    delta_sql: &str,
    _dialect: MaintenanceDialect,
) -> String {
    assert!(
        !key.is_empty(),
        "keyed_fold_candidate_select requires a non-empty key for {table}"
    );
    let unmatched_guard = format!("target.{} IS NULL", key[0]);
    let key_cols = key
        .iter()
        .map(|k| format!("COALESCE(delta.{k}, target.{k}) AS {k}"))
        .collect::<Vec<_>>()
        .join(", ");
    let fold_cols = folds
        .iter()
        .map(|(col, expr)| {
            format!("CASE WHEN {unmatched_guard} THEN delta.{col} ELSE {expr} END AS {col}")
        })
        .collect::<Vec<_>>()
        .join(", ");
    let on = key
        .iter()
        .map(|k| format!("target.{k} = delta.{k}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let select_list = if fold_cols.is_empty() {
        key_cols
    } else {
        format!("{key_cols}, {fold_cols}")
    };
    format!("SELECT {select_list} FROM ({delta_sql}) AS delta LEFT JOIN {table} AS target ON {on}")
}

// ── Decomposed state (rung 2) select augmentation (`docs/specs/
// incremental_shapes.md` §"Decomposed state (rung 2) in keyed models") ────

/// Why [`state_augmented_projection`] could not append the state columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateAugmentRefusal {
    /// `sql` could not be parsed, or its SELECT list could not be located —
    /// fail-closed rather than text-splice blind.
    Unparseable,
}

/// Append one `, <per_partition_expr> AS <name>` select item per
/// `state_columns` to `sql`'s own SELECT list, leaving every other clause
/// (the key/GROUP BY columns, the model's own presented select items, WHERE/
/// FROM/GROUP BY) byte-unchanged. `state_columns` is derived once from the
/// classification (`decomposed_state::DecomposedState::state_columns`
/// across every state-bearing `AggregatorColumn`); the caller applies this
/// to the compiled delta SELECT so the stored table and the delta agree on
/// columns before `CREATE TABLE AS` / `MERGE ... WHEN NOT MATCHED THEN
/// INSERT *` (`docs/specs/incremental_shapes.md` §"Decomposed state (rung 2)
/// in keyed models"). `state_columns.is_empty()` returns `sql` unchanged —
/// the stateless shape every column family admitted before this mechanism
/// existed still produces.
///
/// The insertion point is located via the CST (the last select item's own
/// `text_range`), never a whole-text scan — this emitter is a leaf
/// operation over one already-parsed SELECT, not a second admission pass
/// (`docs/specs/architecture.md` §"Property composition walk rule").
/// Refuses (never mangles the string) when `sql` doesn't parse or its
/// SELECT list can't be located.
pub fn state_augmented_projection(
    sql: &str,
    state_columns: &[crate::analysis::decomposed_state::StateColumn],
) -> Result<String, StateAugmentRefusal> {
    if state_columns.is_empty() {
        return Ok(sql.to_string());
    }
    let parse = smelt_parser::parse(sql);
    let file = smelt_parser::File::cast(parse.syntax()).ok_or(StateAugmentRefusal::Unparseable)?;
    let select = file.select_stmt().ok_or(StateAugmentRefusal::Unparseable)?;
    let list = select
        .select_list()
        .ok_or(StateAugmentRefusal::Unparseable)?;
    let last_item = list
        .items()
        .last()
        .ok_or(StateAugmentRefusal::Unparseable)?;
    let insert_at: usize = last_item.range().end().into();

    let mut additions = String::new();
    for state_col in state_columns {
        additions.push_str(&format!(
            ", {} AS {}",
            state_col.per_partition_expr, state_col.name
        ));
    }
    let mut out = String::with_capacity(sql.len() + additions.len());
    out.push_str(&sql[..insert_at]);
    out.push_str(&additions);
    out.push_str(&sql[insert_at..]);
    Ok(out)
}

// ── Presentation projection (rung 2, `docs/specs/incremental_models.md`
// §"Decomposed state (rung 2) in keyed models" → "Presentation
// projection") ────────────────────────────────────────────────────────

/// Why [`presentation_projection`] could not hide state columns behind a
/// wildcard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresentationRefusal {
    /// `sql` could not be parsed, or its SELECT list could not be located —
    /// fail-closed rather than text-splice blind.
    Unparseable,
    /// A wildcard's relation could not be resolved while a state-bearing
    /// model was in scope. Passing it through unrewritten risks leaking
    /// state columns into the consumer's schema, so this refuses instead of
    /// guessing (`docs/specs/incremental_shapes.md` §"Decomposed state
    /// (rung 2) in keyed models" → "Presentation projection").
    UnresolvableWildcard {
        /// The offending wildcard's own source text (`*` or
        /// `<qualifier>.*`).
        wildcard: String,
    },
    /// The SQL is a pipe query (`pipe_sql.md`) reading a state-bearing
    /// model. This emitter hides state columns behind a SELECT list, and a
    /// pipe query has none to edit, so passing it through would leak them.
    /// Distinguished from [`PresentationRefusal::Unparseable`] because the
    /// SQL parses fine — it is this rewrite that cannot express the hiding.
    PipeQueryOverStateBearingModel,
}

impl std::fmt::Display for PresentationRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PresentationRefusal::Unparseable => {
                write!(f, "SQL could not be parsed for presentation projection")
            }
            PresentationRefusal::PipeQueryOverStateBearingModel => write!(
                f,
                "a pipe query cannot hide the decomposed state columns of the model it reads \
                 behind a presentation projection — rewrite it as a SELECT that names the \
                 columns it presents"
            ),
            PresentationRefusal::UnresolvableWildcard { wildcard } => write!(
                f,
                "wildcard `{wildcard}` could not be resolved to a FROM/JOIN relation while a \
                 state-bearing model is in scope"
            ),
        }
    }
}

/// One relation in a SELECT's `FROM`/`JOIN` clause, as `presentation_
/// projection` needs it: the name a wildcard can qualify it by, and (when
/// it is a `smelt.<path>` reference) the leaf model name `state_bearing`
/// is keyed by.
struct Relation {
    /// The name this relation can be referenced by from the select list —
    /// its explicit/implicit alias, falling back to the leaf model name or
    /// plain identifier when unaliased. `None` only for a relation this
    /// walk cannot name at all (e.g. an unaliased subquery).
    qualifier: Option<String>,
    /// The name `state_bearing` is keyed by: a `smelt.<path>` reference's
    /// leaf segment, or a plain identifier. `None` for a relation with
    /// neither (a subquery), which can never be state-bearing.
    resolved_name: Option<String>,
}

/// Resolve one `TableRef`'s leaf `smelt.<path>` model name (value or
/// call form), mirroring the leaf-segment extraction every other
/// `smelt-logical` walk over a `FROM`/`JOIN` clause already duplicates
/// (`analysis/walk.rs`'s `normalize_table_ref`, `analysis/source_bounds.rs`,
/// `rules/incremental.rs`) — there is no shared helper to call instead, and
/// `smelt-logical` has no dependency on `smelt-db` to borrow one from.
fn table_ref_model_name(table_ref: &smelt_parser::ast::TableRef) -> Option<String> {
    table_ref
        .smelt_path_ref()
        .and_then(|p| p.segments().last().cloned())
        .or_else(|| {
            table_ref
                .smelt_path_call()
                .and_then(|p| p.segments().last().cloned())
        })
}

/// All relations a SELECT's `FROM`/comma-list/`JOIN`s contribute, in source
/// order — the same `table_refs().chain(joins()...)` traversal
/// `analysis/walk.rs`'s `normalize_from` already uses for from-clause
/// enumeration.
fn from_relations(from: &smelt_parser::ast::FromClause) -> Vec<Relation> {
    from.table_refs()
        .chain(from.joins().filter_map(|j| j.table_ref()))
        .map(|table_ref| {
            let resolved_name = table_ref_model_name(&table_ref).or_else(|| table_ref.identifier());
            let qualifier = table_ref.alias().or_else(|| resolved_name.clone());
            Relation {
                qualifier,
                resolved_name,
            }
        })
        .collect()
}

/// Whether any table this pipe query reads — its entry `FROM`, a `\|> JOIN`
/// stage's right side, or a table inside a CTE/subquery — is a model
/// `state_bearing` names.
///
/// Traverses every `TABLE_REF` under the pipe query rather than the entry
/// `FROM` alone: a pipe query reaching a state-bearing model from any of
/// those positions would present its state columns just the same, and this
/// decides a refusal, so it errs wide.
fn pipe_reads_state_bearing(
    pipe: &smelt_parser::ast::PipeQuery,
    state_bearing: &std::collections::BTreeMap<String, Vec<String>>,
) -> bool {
    pipe.syntax()
        .descendants()
        .filter_map(smelt_parser::ast::TableRef::cast)
        .filter_map(|table_ref| table_ref_model_name(&table_ref).or_else(|| table_ref.identifier()))
        .any(|name| state_bearing.contains_key(&name))
}

/// Rewrite `sql`'s wildcard select items so a state-bearing model's
/// `__part` state columns never reach a consumer's schema: a wildcard over
/// a relation `state_bearing` names is expanded to that model's presented
/// columns (`state_bearing`'s values, in schema order); a wildcard over a
/// relation `state_bearing` does not name is left byte-unchanged.
/// `state_bearing` maps model name → presented column names — its values
/// come from the public schema (`UpstreamSchemas::models` at the caller),
/// its keys from the set of models classified as state-bearing; this
/// function invents no new source of truth for "which columns are
/// presented".
///
/// Returns `sql` byte-identical when no relation in scope is
/// state-bearing (`state_bearing.is_empty()` or none of its keys appear in
/// `sql`'s `FROM`/`JOIN`) — the parity path every project not using
/// decomposed state still takes. Refuses
/// ([`PresentationRefusal::UnresolvableWildcard`]) rather than passing a
/// wildcard through unexpanded when a state-bearing relation is in scope
/// and the wildcard's own relation cannot be resolved (an unknown
/// qualifier, or a bare `*` over an unnameable relation) — a silent
/// pass-through there would leak state columns into the consumer's schema.
///
/// The rewrite locates each wildcard select item via its own CST
/// `range()`, never a whole-text scan for `*` — this emitter is a leaf
/// operation over one already-parsed SELECT (`docs/specs/architecture.md`
/// §"Property composition walk rule"), so a `*` inside a string literal
/// (wrapped in an `EXPRESSION` node, `SelectItem::is_wildcard()` returns
/// `false` for it) is never touched.
pub fn presentation_projection(
    sql: &str,
    state_bearing: &std::collections::BTreeMap<String, Vec<String>>,
) -> Result<String, PresentationRefusal> {
    // Nothing state-bearing anywhere in the project: there are no columns to
    // hide, so this rewrite has no work to do and must not judge the SQL's
    // form. It runs on *every* compiled model, so parsing first and refusing
    // on a non-SELECT body (a pipe query) failed those models outright.
    if state_bearing.is_empty() {
        return Ok(sql.to_string());
    }

    let parse = smelt_parser::parse(sql);
    let file = smelt_parser::File::cast(parse.syntax()).ok_or(PresentationRefusal::Unparseable)?;
    let Some(select) = file.select_stmt() else {
        // A pipe query parses cleanly but carries no SELECT list to edit.
        // Only refuse when it actually reads a state-bearing model.
        if file
            .pipe_query()
            .is_some_and(|pipe| pipe_reads_state_bearing(&pipe, state_bearing))
        {
            return Err(PresentationRefusal::PipeQueryOverStateBearingModel);
        }
        return Ok(sql.to_string());
    };
    let list = select
        .select_list()
        .ok_or(PresentationRefusal::Unparseable)?;
    let relations: Vec<Relation> = select
        .from_clause()
        .map(|from| from_relations(&from))
        .unwrap_or_default();

    let any_state_bearing = relations
        .iter()
        .any(|r| matches_state_bearing(r, state_bearing));
    if !any_state_bearing {
        return Ok(sql.to_string());
    }

    let mut out = String::with_capacity(sql.len());
    let mut last_end: usize = 0;
    for item in list.items() {
        let replacement = if let Some(qualifier) = item.qualified_wildcard_target() {
            match relations
                .iter()
                .find(|r| r.qualifier.as_deref() == Some(qualifier.as_str()))
            {
                Some(rel) => match state_bearing_columns(rel, state_bearing) {
                    Some(cols) => Some(
                        cols.iter()
                            .map(|c| format!("{qualifier}.{c}"))
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                    None => None,
                },
                None => {
                    return Err(PresentationRefusal::UnresolvableWildcard {
                        wildcard: sql[item.range()].to_string(),
                    });
                }
            }
        } else if item.is_wildcard() {
            Some(
                expand_bare_wildcard(&relations, state_bearing).ok_or_else(|| {
                    PresentationRefusal::UnresolvableWildcard {
                        wildcard: sql[item.range()].to_string(),
                    }
                })?,
            )
        } else {
            None
        };

        if let Some(replacement) = replacement {
            let start: usize = item.range().start().into();
            let end: usize = item.range().end().into();
            out.push_str(&sql[last_end..start]);
            out.push_str(&replacement);
            last_end = end;
        }
    }
    out.push_str(&sql[last_end..]);
    Ok(out)
}

fn matches_state_bearing(
    rel: &Relation,
    state_bearing: &std::collections::BTreeMap<String, Vec<String>>,
) -> bool {
    rel.resolved_name
        .as_deref()
        .is_some_and(|n| state_bearing.contains_key(n))
}

fn state_bearing_columns<'a>(
    rel: &Relation,
    state_bearing: &'a std::collections::BTreeMap<String, Vec<String>>,
) -> Option<&'a Vec<String>> {
    rel.resolved_name
        .as_deref()
        .and_then(|n| state_bearing.get(n))
}

/// Expand a bare `*` given the relations in scope. A single relation
/// expands to its bare (unqualified) presented column list; multiple
/// relations expand per-relation — a state-bearing relation to its
/// qualified presented columns, a non-state-bearing (or non-state-bearing-
/// unresolvable) relation to `<qualifier>.*`. `None` means the wildcard
/// cannot be resolved (a relation this walk cannot name, still in scope
/// alongside a state-bearing one) and the caller must refuse.
fn expand_bare_wildcard(
    relations: &[Relation],
    state_bearing: &std::collections::BTreeMap<String, Vec<String>>,
) -> Option<String> {
    if relations.len() == 1 {
        let rel = &relations[0];
        return match state_bearing_columns(rel, state_bearing) {
            Some(cols) => Some(cols.join(", ")),
            // Only reachable if this sole relation isn't state-bearing —
            // but the caller only reaches `expand_bare_wildcard` once it
            // has already established some relation in scope IS
            // state-bearing, and with one relation that must be this one.
            // Kept as a refusal (never a silent unchanged copy) rather
            // than an `unreachable!()`, so a future relaxation of the
            // caller's gate fails loud instead of miscompiling.
            None => None,
        };
    }
    let mut parts = Vec::with_capacity(relations.len());
    for rel in relations {
        match state_bearing_columns(rel, state_bearing) {
            Some(cols) => {
                let qualifier = rel.qualifier.as_deref()?;
                for c in cols {
                    parts.push(format!("{qualifier}.{c}"));
                }
            }
            None => {
                let qualifier = rel.qualifier.as_deref()?;
                parts.push(format!("{qualifier}.*"));
            }
        }
    }
    Some(parts.join(", "))
}

#[cfg(test)]
mod keyed_fold_candidate_select_tests {
    use super::super::staged::emit_staged_candidate_conditional;
    use super::*;

    fn folds() -> Vec<(String, String)> {
        vec![(
            "event_count".to_string(),
            "target.event_count + delta.event_count".to_string(),
        )]
    }

    #[test]
    fn keyed_fold_candidate_select_folds_stored_state_against_the_delta() {
        let sql = keyed_fold_candidate_select(
            "main.user_stats",
            &["user_id".to_string()],
            &folds(),
            "SELECT user_id, COUNT(*) AS event_count FROM events GROUP BY user_id",
            MaintenanceDialect::DuckDb,
        );

        // A matched key's candidate row resolves to the fold's own combine
        // expression, exactly as emit_keyed_fold's matched arm does.
        assert!(
            sql.contains("ELSE target.event_count + delta.event_count END AS event_count"),
            "expected the fold's combine expression in the matched (ELSE) arm, got: {sql}"
        );
        assert!(
            sql.contains("LEFT JOIN main.user_stats AS target ON target.user_id = delta.user_id"),
            "expected a LEFT JOIN of the delta to the target on the key, got: {sql}"
        );
    }

    #[test]
    fn keyed_fold_candidate_select_carries_keys_absent_from_the_target() {
        let sql = keyed_fold_candidate_select(
            "main.user_stats",
            &["user_id".to_string()],
            &folds(),
            "SELECT user_id, COUNT(*) AS event_count FROM events GROUP BY user_id",
            MaintenanceDialect::DuckDb,
        );

        // A delta-only key (no stored row) resolves to its own raw delta
        // value, matching WHEN NOT MATCHED THEN INSERT's whole-row insert.
        assert!(
            sql.contains("CASE WHEN target.user_id IS NULL THEN delta.event_count ELSE"),
            "expected the delta-only THEN arm to carry the raw delta value, got: {sql}"
        );
    }

    #[test]
    fn keyed_fold_candidate_select_feeds_the_staged_emitter_unchanged() {
        let candidate_select = keyed_fold_candidate_select(
            "main.user_stats",
            &["user_id".to_string()],
            &folds(),
            "SELECT user_id, COUNT(*) AS event_count FROM events GROUP BY user_id",
            MaintenanceDialect::DuckDb,
        );

        let group = emit_staged_candidate_conditional(
            "main.user_stats",
            "__smelt_staged_user_stats",
            &["user_id".to_string()],
            &candidate_select,
            &["event_count".to_string()],
            MaintenanceDialect::DuckDb,
        );

        assert!(group.transactional);
        assert_eq!(group.statements.len(), 5);
        assert_eq!(
            group.statements[2].sql,
            format!(
                "DELETE FROM main.user_stats USING __smelt_staged_user_stats WHERE \
                 main.user_stats.user_id = __smelt_staged_user_stats.user_id AND \
                 (main.user_stats.event_count IS DISTINCT FROM \
                 __smelt_staged_user_stats.event_count)"
            )
        );
    }

    #[test]
    #[should_panic(expected = "requires a non-empty key")]
    fn keyed_fold_candidate_select_panics_on_empty_key() {
        keyed_fold_candidate_select(
            "main.user_stats",
            &[],
            &folds(),
            "SELECT user_id, COUNT(*) AS event_count FROM events GROUP BY user_id",
            MaintenanceDialect::DuckDb,
        );
    }
}
