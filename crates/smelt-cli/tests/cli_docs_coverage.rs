//! Standing checklist gate for docs-site CLI-surface coverage
//! (`docs/specs/incremental_models.md` §Known Divergences — "docs-site coverage of
//! the plan's CLI surface is partial"). Rather than a one-time audit, this test
//! walks the real `Commands`/`DocsCommands` enums and every `*Args` struct's long
//! flags in `src/main.rs`, and fails if a command or flag is undocumented in
//! `docs-site/docs/reference/cli.md` and not named in the allowlist below.

use std::fs;
use std::path::PathBuf;

/// Commands or `command:--flag` pairs deliberately excluded from documentation,
/// each with a one-line reason. Two-sided: `allowlist_has_no_stale_entries`
/// fails if an entry here no longer exists, or is now documented.
const UNDOCUMENTED_BY_DESIGN: &[&str] = &[];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn main_rs() -> String {
    fs::read_to_string(repo_root().join("crates/smelt-cli/src/main.rs")).unwrap()
}

fn cli_md() -> String {
    fs::read_to_string(repo_root().join("docs-site/docs/reference/cli.md")).unwrap()
}

/// Extract the `{ ... }` body of `enum <enum_name>` via brace counting (handles
/// nested struct-variants like `Show { topic: String }`).
fn extract_enum_body(src: &str, enum_name: &str) -> String {
    let marker = format!("enum {enum_name} {{");
    let start = src
        .find(&marker)
        .unwrap_or_else(|| panic!("`enum {enum_name}` not found in main.rs"));
    let body_start = start + marker.len();
    let bytes = src.as_bytes();
    let mut depth = 1i32;
    let mut i = body_start;
    while depth > 0 {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    src[body_start..i - 1].to_string()
}

/// Extract the `{ ... }` body of `struct <struct_name>` via brace counting.
fn extract_struct_body(src: &str, struct_name: &str) -> Option<String> {
    let marker = format!("struct {struct_name} {{");
    let start = src.find(&marker)?;
    let body_start = start + marker.len();
    let bytes = src.as_bytes();
    let mut depth = 1i32;
    let mut i = body_start;
    while depth > 0 {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    Some(src[body_start..i - 1].to_string())
}

/// One enum variant: its name, and (if a tuple variant) the `*Args` struct it wraps.
struct Variant {
    name: String,
    args_struct: Option<String>,
}

/// Parse top-level variant declarations (exactly 4-space indent) out of an enum body.
/// Rustfmt's consistent nesting increment means filtering on indent alone (rather
/// than brace-depth tracking) correctly skips a struct-variant's inner fields.
fn parse_variants(body: &str) -> Vec<Variant> {
    let mut variants = vec![];
    for line in body.lines() {
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim();
        if indent != 4 || trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("///") || trimmed.starts_with("#[") {
            continue;
        }
        let first = trimmed.chars().next().unwrap();
        if !first.is_ascii_uppercase() {
            continue;
        }
        let name: String = trimmed
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        let rest = &trimmed[name.len()..];
        let args_struct = rest
            .trim_start()
            .strip_prefix('(')
            .and_then(|s| s.split(')').next())
            .map(|inner| inner.trim().to_string());
        variants.push(Variant { name, args_struct });
    }
    variants
}

/// Every `#[arg(long...)]` flag declared in a struct body, resolved to its
/// `--kebab-case` spelling (explicit `long = "..."` wins; otherwise the field
/// name on the following line, snake_case -> kebab-case).
fn extract_flags(body: &str) -> Vec<String> {
    let lines: Vec<&str> = body.lines().collect();
    let mut flags = vec![];
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if !trimmed.starts_with("#[arg(") || !trimmed.contains("long") {
            continue;
        }
        if let Some(after) = trimmed.split("long = \"").nth(1) {
            let name = after.split('"').next().unwrap_or("");
            if !name.is_empty() {
                flags.push(name.to_string());
                continue;
            }
        }
        // Bare `long` (no explicit spelling): derive from the next line's field name.
        if let Some(next) = lines.get(i + 1) {
            let field = next.trim().split(':').next().unwrap_or("").trim();
            if !field.is_empty() && field.chars().all(|c| c.is_alphanumeric() || c == '_') {
                flags.push(field.replace('_', "-"));
            }
        }
    }
    flags
}

/// (full command name e.g. "docs generate", declared long flags for that command).
fn all_commands() -> Vec<(String, Vec<String>)> {
    let src = main_rs();
    let mut out = vec![];

    // Root-level global flags, under the pseudo-command "smelt".
    let cli_body = extract_struct_body(&src, "Cli").expect("`struct Cli` not found");
    out.push(("".to_string(), extract_flags(&cli_body)));

    let commands_body = extract_enum_body(&src, "Commands");
    for variant in parse_variants(&commands_body) {
        let full_name = variant.name.to_lowercase();
        if variant.name == "Docs" {
            let docs_body = extract_enum_body(&src, "DocsCommands");
            for sub in parse_variants(&docs_body) {
                let sub_name = format!("docs {}", sub.name.to_lowercase());
                let flags = sub
                    .args_struct
                    .as_deref()
                    .and_then(|s| extract_struct_body(&src, s))
                    .map(|b| extract_flags(&b))
                    .unwrap_or_default();
                out.push((sub_name, flags));
            }
            continue;
        }
        let flags = variant
            .args_struct
            .as_deref()
            .and_then(|s| extract_struct_body(&src, s))
            .map(|b| extract_flags(&b))
            .unwrap_or_default();
        out.push((full_name, flags));
    }
    out
}

#[test]
fn every_command_is_documented() {
    let doc = cli_md();
    let mut missing = vec![];
    for (name, _flags) in all_commands() {
        if name.is_empty() {
            continue; // root pseudo-command has no "## smelt " heading of its own
        }
        let heading = format!("## smelt {name}");
        let allowlisted = UNDOCUMENTED_BY_DESIGN.contains(&name.as_str());
        if !doc.contains(&heading) && !allowlisted {
            missing.push(name);
        }
    }
    assert!(
        missing.is_empty(),
        "commands missing a `## smelt <name>` heading in docs-site/docs/reference/cli.md \
         (or an UNDOCUMENTED_BY_DESIGN entry): {missing:?}"
    );
}

#[test]
fn every_long_flag_is_documented() {
    let doc = cli_md();
    let mut missing = vec![];
    for (name, flags) in all_commands() {
        for flag in flags {
            let literal = format!("--{flag}");
            let key = format!("{name}:--{flag}");
            let allowlisted = UNDOCUMENTED_BY_DESIGN.contains(&key.as_str());
            if !doc.contains(&literal) && !allowlisted {
                missing.push(key);
            }
        }
    }
    assert!(
        missing.is_empty(),
        "flags not documented verbatim in docs-site/docs/reference/cli.md \
         (or missing an UNDOCUMENTED_BY_DESIGN entry, as \"command:--flag\"): {missing:?}"
    );
}

#[test]
fn allowlist_has_no_stale_entries() {
    let doc = cli_md();
    let commands = all_commands();
    let command_names: Vec<&str> = commands.iter().map(|(n, _)| n.as_str()).collect();

    for &entry in UNDOCUMENTED_BY_DESIGN {
        if let Some((cmd, flag)) = entry.split_once(":--") {
            // A "command:--flag" entry: the command must still exist, still
            // declare that flag, and the flag must still be undocumented.
            let owning = commands.iter().find(|(n, _)| n == cmd);
            match owning {
                None => panic!("allowlist entry `{entry}` names a command that no longer exists"),
                Some((_, flags)) => {
                    assert!(
                        flags.iter().any(|f| f == flag),
                        "allowlist entry `{entry}` names a flag the command no longer declares"
                    );
                    let literal = format!("--{flag}");
                    assert!(
                        !doc.contains(&literal),
                        "allowlist entry `{entry}` is now documented in cli.md — delete the entry"
                    );
                }
            }
        } else {
            // A bare command entry.
            assert!(
                command_names.contains(&entry),
                "allowlist entry `{entry}` names a command that no longer exists"
            );
            let heading = format!("## smelt {entry}");
            assert!(
                !doc.contains(&heading),
                "allowlist entry `{entry}` is now documented in cli.md — delete the entry"
            );
        }
    }
}
