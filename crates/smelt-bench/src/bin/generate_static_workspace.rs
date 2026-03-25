//! Generate a static workspace of 2000 models for examples/huge/.
//!
//! Usage: cargo run -p smelt-bench --bin generate_static_workspace -- [output_dir]
//!
//! Defaults to examples/huge/ relative to the repo root.

use std::path::PathBuf;

use smelt_bench::{generate_workspace_to_path, GraphSpec};

fn main() -> anyhow::Result<()> {
    let output = match std::env::args().nth(1) {
        Some(path) => PathBuf::from(path),
        None => {
            // Default: examples/huge/ relative to repo root
            let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            manifest_dir
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("examples/huge")
        }
    };

    let spec = GraphSpec::default();
    println!(
        "Generating {} models into {}...",
        spec.total_models(),
        output.display()
    );

    let models = generate_workspace_to_path(&spec, &output)?;
    println!("Generated {} models successfully.", models.len());

    Ok(())
}
