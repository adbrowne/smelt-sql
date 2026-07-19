//! `smelt init` — non-interactive scaffolder.
//!
//! Writes a minimal, working smelt project to a target directory: a
//! `smelt.yml`, a `models/` directory with one example model, one seed CSV
//! under `seeds/`, and a `.gitignore` excluding `.smelt/` and the database
//! file. Every file written is a fixed, deterministic template embedded at
//! compile time via `include_str!` from `templates/init/` — there is no
//! wizard and no flag that changes what gets scaffolded beyond the target
//! directory.
//!
//! Template content mirrors `docs-site/docs/getting-started/quickstart.md`
//! so the docs and the scaffold cannot drift apart.
//!
//! Spec: `docs/specs/cli.md` §"`smelt init` — non-interactive scaffolder".

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::InitArgs;

const SMELT_YML: &str = include_str!("../../templates/init/smelt.yml");
const GITIGNORE: &str = include_str!("../../templates/init/.gitignore");
const EXAMPLE_MODEL: &str = include_str!("../../templates/init/models/orders_summary.sql");
const SEED_CSV: &str = include_str!("../../templates/init/seeds/raw_orders.csv");

/// Errors specific to `smelt init`.
#[derive(Debug, Error)]
pub enum InitError {
    /// The target directory already contains a `smelt.yml`. `smelt init`
    /// never overwrites or merges an existing project — there is
    /// deliberately no `--force` flag; re-run guidance is the only
    /// remediation (`docs/specs/cli.md` item 15).
    #[error(
        "Directory already contains a smelt.yml: {path}\n\
         smelt init refuses to overwrite an existing project.\n\
         Run `smelt init` in a different (empty) directory, or remove {path} and re-run `smelt init` here."
    )]
    AlreadyExists { path: PathBuf },
}

/// Classify an `smelt init` error into the exit-code contract
/// (`docs/specs/cli.md` §"Exit codes"): `2` for the non-empty-directory
/// refusal (a usage error — the fix is a different directory, not a retry
/// of the same command), `1` for anything else (e.g. an I/O failure while
/// writing the scaffold).
pub fn exit_code_for(err: &anyhow::Error) -> u8 {
    if err.downcast_ref::<InitError>().is_some() {
        2
    } else {
        1
    }
}

/// Run `smelt init [DIR]`.
pub fn run(args: InitArgs) -> Result<()> {
    let dir = args.dir.unwrap_or_else(|| PathBuf::from("."));

    let smelt_yml_path = dir.join("smelt.yml");
    if smelt_yml_path.exists() {
        return Err(InitError::AlreadyExists {
            path: smelt_yml_path,
        }
        .into());
    }

    fs::create_dir_all(dir.join("models"))
        .with_context(|| format!("failed to create {}", dir.join("models").display()))?;
    fs::create_dir_all(dir.join("seeds"))
        .with_context(|| format!("failed to create {}", dir.join("seeds").display()))?;

    write_new(&smelt_yml_path, SMELT_YML)?;
    write_new(&dir.join(".gitignore"), GITIGNORE)?;
    write_new(
        &dir.join("models").join("orders_summary.sql"),
        EXAMPLE_MODEL,
    )?;
    write_new(&dir.join("seeds").join("raw_orders.csv"), SEED_CSV)?;

    println!("Scaffolded a new smelt project in {}", dir.display());
    println!();
    println!("Next steps:");
    println!("  cd {}", dir.display());
    println!("  smelt build");

    Ok(())
}

fn write_new(path: &Path, contents: &str) -> Result<()> {
    fs::write(path, contents).with_context(|| format!("failed to write {}", path.display()))
}
