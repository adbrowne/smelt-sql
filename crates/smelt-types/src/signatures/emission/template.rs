use super::expr_kind::ExprKind;
use super::position_rewrite::Position;
use crate::signatures::{SigParam, Signature};

/// Why an [`super::Emission::Template`] row failed registry-construction
/// validation (`docs/specs/multi_backend.md` §"Template emission"). Every
/// variant names the malformed template and the signature it was declared
/// against, so a bad registry seed fails loudly with enough context to fix it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateError {
    /// A placeholder `{n}` named an index at or beyond the signature's fixed
    /// (non-variadic) arity.
    IndexOutOfRange {
        signature: String,
        index: usize,
        arity: usize,
    },
    /// A fixed parameter had no placeholder referencing it — the template
    /// would silently drop an argument the call supplies.
    ArgumentUnreferenced { signature: String, index: usize },
    /// The template's literal parentheses (outside `{n}` placeholders) do not
    /// balance.
    UnbalancedParens { signature: String },
    /// A template stated at `Window`/`WholePartitionWindow`, or at `Any` for
    /// a signature whose `kind` is `Agg`/`Window`, is not call-shaped —
    /// `(expr) OVER (…)` is not valid SQL, so the substituted form must be a
    /// single call.
    NonCallAtWindowPosition { signature: String },
    /// A template was declared on a signature with a trailing variadic
    /// parameter — a fixed `{n}` placeholder cannot name a variadic tail.
    VariadicSignature { signature: String },
}

impl std::fmt::Display for TemplateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TemplateError::IndexOutOfRange {
                signature,
                index,
                arity,
            } => write!(
                f,
                "template for `{signature}` references index {{{index}}}, but the signature's fixed arity is {arity}"
            ),
            TemplateError::ArgumentUnreferenced { signature, index } => write!(
                f,
                "template for `{signature}` never references fixed parameter {index} — it would silently drop an argument"
            ),
            TemplateError::UnbalancedParens { signature } => write!(
                f,
                "template for `{signature}` has unbalanced parentheses"
            ),
            TemplateError::NonCallAtWindowPosition { signature } => write!(
                f,
                "template for `{signature}` is stated at a window position but is not call-shaped — `(expr) OVER (…)` is not valid SQL"
            ),
            TemplateError::VariadicSignature { signature } => write!(
                f,
                "template for `{signature}` is declared on a variadic signature — a placeholder cannot name a variadic tail"
            ),
        }
    }
}

impl std::error::Error for TemplateError {}

/// Structural (never textual) test for "the template's outermost form is a
/// single call": an identifier followed by a parenthesised group that closes
/// exactly at the end of the string — `MOD({0}, {1})`, not `{0} - {1}` or
/// `DAYOFWEEK({0}) - 1`.
///
/// Shared by [`validate_template`] (a non-call template is refused at a
/// window position) and the printer's interpreter (a non-call template's
/// substituted output is wrapped in parentheses so it composes correctly in
/// the surrounding expression; a call-shaped one never needs argument-level
/// wrapping, since a function call's comma-separated arguments are already
/// unambiguously delimited).
pub fn is_call_shaped_template(template: &str) -> bool {
    let t = template.trim();
    let Some(open) = t.find('(') else {
        return false;
    };
    if !t.ends_with(')') || open == t.len() - 1 {
        return false;
    }
    let name = &t[..open];
    if name.is_empty()
        || !name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return false;
    }
    // The opening paren at `open` must close exactly at the template's last
    // byte — a trailing operator after a balanced call, e.g. `MOD(a, b) + 1`,
    // is not call-shaped even though it ends in `)`.
    let mut depth = 0i32;
    for (i, c) in t.char_indices().skip(open) {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return i == t.len() - 1;
                }
            }
            _ => {}
        }
    }
    false
}

/// Validate an [`super::Emission::Template`] row at registry-construction time
/// (`docs/specs/multi_backend.md` §"Template emission"). Pure — no I/O, no
/// panics; the registry seed converts an `Err` into a build-time panic so a
/// malformed template is a compile-adjacent failure, never a runtime one.
pub fn validate_template(
    template: &'static str,
    sig: &Signature,
    position: Position,
) -> Result<(), TemplateError> {
    let variadic = matches!(sig.params.last(), Some(SigParam::Variadic(_)));
    if variadic {
        return Err(TemplateError::VariadicSignature {
            signature: sig.name.clone(),
        });
    }
    let arity = sig.params.len();

    let mut depth = 0i32;
    let mut referenced = vec![false; arity];
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                depth -= 1;
                if depth < 0 {
                    return Err(TemplateError::UnbalancedParens {
                        signature: sig.name.clone(),
                    });
                }
                i += 1;
            }
            b'{' => {
                let Some(rel_end) = template[i..].find('}') else {
                    return Err(TemplateError::UnbalancedParens {
                        signature: sig.name.clone(),
                    });
                };
                let end = i + rel_end;
                let idx: usize =
                    template[i + 1..end]
                        .parse()
                        .map_err(|_| TemplateError::IndexOutOfRange {
                            signature: sig.name.clone(),
                            index: 0,
                            arity,
                        })?;
                if idx >= arity {
                    return Err(TemplateError::IndexOutOfRange {
                        signature: sig.name.clone(),
                        index: idx,
                        arity,
                    });
                }
                referenced[idx] = true;
                i = end + 1;
            }
            _ => i += 1,
        }
    }
    if depth != 0 {
        return Err(TemplateError::UnbalancedParens {
            signature: sig.name.clone(),
        });
    }
    if let Some(index) = referenced.iter().position(|seen| !seen) {
        return Err(TemplateError::ArgumentUnreferenced {
            signature: sig.name.clone(),
            index,
        });
    }

    let call_shaped = is_call_shaped_template(template);
    let window_position = matches!(position, Position::Window | Position::WholePartitionWindow)
        || (position == Position::Any && matches!(sig.kind, ExprKind::Agg | ExprKind::Window));
    if window_position && !call_shaped {
        return Err(TemplateError::NonCallAtWindowPosition {
            signature: sig.name.clone(),
        });
    }

    Ok(())
}
