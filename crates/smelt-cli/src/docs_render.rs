use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

use crate::docs::Catalog;

/// Render the catalog as a single JSON file.
pub fn render_json(catalog: &Catalog, output_dir: &Path) -> Result<()> {
    fs::create_dir_all(output_dir).with_context(|| {
        format!(
            "Failed to create output directory: {}",
            output_dir.display()
        )
    })?;

    let json = serde_json::to_string_pretty(catalog)
        .with_context(|| "Failed to serialize catalog to JSON")?;

    let path = output_dir.join("catalog.json");
    fs::write(&path, json).with_context(|| format!("Failed to write {}", path.display()))?;

    Ok(())
}

/// Render the catalog as a directory of Markdown files.
pub fn render_markdown(catalog: &Catalog, output_dir: &Path) -> Result<()> {
    fs::create_dir_all(output_dir).with_context(|| {
        format!(
            "Failed to create output directory: {}",
            output_dir.display()
        )
    })?;

    let models_dir = output_dir.join("models");
    fs::create_dir_all(&models_dir).with_context(|| {
        format!(
            "Failed to create models directory: {}",
            models_dir.display()
        )
    })?;

    // Write index page
    let index = render_index(catalog);
    let index_path = output_dir.join("index.md");
    fs::write(&index_path, index)
        .with_context(|| format!("Failed to write {}", index_path.display()))?;

    // Write per-model pages
    for model in catalog.models.values() {
        let page = render_model_page(model);
        let page_path = models_dir.join(format!("{}.md", model.name));
        fs::write(&page_path, page)
            .with_context(|| format!("Failed to write {}", page_path.display()))?;
    }

    Ok(())
}

fn render_index(catalog: &Catalog) -> String {
    let mut out = String::new();

    out.push_str(&format!("# {}\n\n", catalog.project.name));
    out.push_str(&format!(
        "**{}** models | Generated {}\n\n",
        catalog.project.model_count, catalog.project.generated_at
    ));

    // Model table
    out.push_str("## Models\n\n");
    out.push_str("| Model | Materialization | Owner | Description |\n");
    out.push_str("|-------|----------------|-------|-------------|\n");

    for name in &catalog.execution_order {
        if let Some(model) = catalog.models.get(name) {
            let owner = model.owner.as_deref().unwrap_or("");
            let desc = model.description.as_deref().unwrap_or("");
            out.push_str(&format!(
                "| [{}](models/{}.md) | {} | {} | {} |\n",
                model.name, model.name, model.materialization, owner, desc
            ));
        }
    }

    // Tag index
    if !catalog.tag_index.is_empty() {
        out.push_str("\n## Tags\n\n");
        for (tag, model_names) in &catalog.tag_index {
            let links: Vec<String> = model_names
                .iter()
                .map(|n| format!("[{}](models/{}.md)", n, n))
                .collect();
            out.push_str(&format!("- **{}**: {}\n", tag, links.join(", ")));
        }
    }

    out
}

pub fn render_model_page(model: &crate::docs::CatalogModel) -> String {
    let mut out = String::new();

    out.push_str(&format!("# {}\n\n", model.name));

    if let Some(ref desc) = model.description {
        out.push_str(&format!("{}\n\n", desc));
    }

    // Metadata
    out.push_str(&format!("**Materialization**: {}\n", model.materialization));
    if let Some(ref owner) = model.owner {
        out.push_str(&format!("**Owner**: {}\n", owner));
    }
    if !model.tags.is_empty() {
        out.push_str(&format!("**Tags**: {}\n", model.tags.join(", ")));
    }
    // Generator-emitted models: surface generator file and ModelDef name.
    if let Some(smelt_core::ModelOriginKind::Generated {
        ref generator_file,
        ref generator_name,
    }) = model.origin
    {
        out.push_str(&format!(
            "**Source**: {} ({})\n",
            generator_file, generator_name
        ));
    }
    out.push('\n');

    // Columns
    if !model.columns.is_empty() {
        out.push_str("## Columns\n\n");
        out.push_str("| Column | Type | Nullable | Description | Tests |\n");
        out.push_str("|--------|------|----------|-------------|-------|\n");

        for col in &model.columns {
            let dtype = col.data_type.as_deref().unwrap_or("unknown");
            let nullable = match col.nullable {
                Some(true) => "yes",
                Some(false) => "no",
                None => "?",
            };
            let desc = col.description.as_deref().unwrap_or("");
            let tests = if col.tests.is_empty() {
                String::new()
            } else {
                col.tests.join(", ")
            };
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                col.name, dtype, nullable, desc, tests
            ));
        }
        out.push('\n');
    }

    // Dependencies
    if !model.upstream.is_empty() {
        out.push_str("## Upstream\n\n");
        for dep in &model.upstream {
            out.push_str(&format!("- [{}]({}.md)\n", dep, dep));
        }
        out.push('\n');
    }

    if !model.downstream.is_empty() {
        out.push_str("## Downstream\n\n");
        for dep in &model.downstream {
            out.push_str(&format!("- [{}]({}.md)\n", dep, dep));
        }
        out.push('\n');
    }

    // Incremental config
    if let Some(ref inc) = model.incremental {
        out.push_str("## Incremental\n\n");
        out.push_str(&format!("- **Granularity**: {}\n", inc.granularity));
        out.push_str(&format!(
            "- **Partition column**: {}\n",
            inc.partition_column
        ));
        out.push_str(&format!(
            "- **Event time column**: {}\n",
            inc.event_time_column
        ));
        if !inc.unique_key.is_empty() {
            out.push_str(&format!(
                "- **Unique key**: {}\n",
                inc.unique_key.join(", ")
            ));
        }
        out.push('\n');
    }

    out
}
