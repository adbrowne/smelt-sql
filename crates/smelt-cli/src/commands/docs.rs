use anyhow::{Context, Result};
use include_dir::{include_dir, Dir};
use smelt_cli::docs::TestRef;
use smelt_cli::{
    discover_emitted_model_files, discover_python_models, find_project_root, init_db,
    parse_selector, Config, ModelDiscovery, ModelFile, SourcesConfig,
};
use smelt_core::graph::DependencyGraph;
use std::collections::HashMap;

use crate::DocsGenerateArgs;

/// User-facing markdown docs, embedded at compile time so `smelt docs show`
/// works from the installed wheel without a network round-trip and is pinned
/// to the exact CLI version the user installed.
static USER_DOCS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../docs-site/docs");

fn collect_topics(dir: &Dir<'_>, out: &mut Vec<String>) {
    for file in dir.files() {
        let path = file.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let topic = path.with_extension("");
            out.push(topic.to_string_lossy().into_owned());
        }
    }
    for sub in dir.dirs() {
        collect_topics(sub, out);
    }
}

fn list_topics() -> Vec<String> {
    let mut topics = Vec::new();
    collect_topics(&USER_DOCS, &mut topics);
    topics.sort();
    topics
}

fn lookup_topic(topic: &str) -> Option<&'static include_dir::File<'static>> {
    let trimmed = topic.trim_end_matches(".md");
    let candidate = format!("{}.md", trimmed);
    USER_DOCS.get_file(&candidate)
}

pub fn list() -> Result<()> {
    for topic in list_topics() {
        println!("{}", topic);
    }
    Ok(())
}

pub fn show(topic: &str) -> Result<()> {
    let file = lookup_topic(topic).with_context(|| {
        let topics = list_topics();
        let near: Vec<&String> = topics
            .iter()
            .filter(|t| t.contains(topic) || topic.contains(t.as_str()))
            .take(5)
            .collect();
        if near.is_empty() {
            format!(
                "Unknown docs topic: '{}'. Run `smelt docs list` to see available topics.",
                topic
            )
        } else {
            let suggestions = near
                .iter()
                .map(|t| format!("  {}", t))
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "Unknown docs topic: '{}'. Did you mean:\n{}",
                topic, suggestions
            )
        }
    })?;
    let body = file
        .contents_utf8()
        .with_context(|| format!("Docs page '{}' is not valid UTF-8", topic))?;
    print!("{}", body);
    Ok(())
}

pub fn path() -> Result<()> {
    println!(
        "Docs are embedded in the smelt binary (no on-disk path).\n\
         Use `smelt docs list` to see available topics and `smelt docs show <topic>` to read one."
    );
    Ok(())
}

pub async fn generate(args: DocsGenerateArgs) -> Result<()> {
    let project_dir = find_project_root(&args.project_dir)
        .with_context(|| format!("Failed to find project root from {:?}", args.project_dir))?;

    let config =
        Config::load(&project_dir).with_context(|| "Failed to load smelt.yml configuration")?;

    let sources = SourcesConfig::load(&project_dir).ok();

    // Seeds are valid `smelt.ref()` targets (bug #2 in 20260417 follow-up).
    let _seeds = smelt_core::discover_seed_infos(&project_dir, &config.paths);

    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let sql_models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;

    // Build Salsa DB from all raw SQL files (including generator files) so
    // the emitted-models pipeline can run via `smelt_db::emitted_models()`.
    let db_emitted = init_db(&project_dir, &sql_models);

    // Discover generator-emitted models and their provenance.
    let (emitted_model_files, origins) =
        discover_emitted_model_files(&db_emitted, &project_dir, &config.paths);

    // Build the model list:
    //   - Exclude generator files (.gen.sql) from the hand-authored set so they
    //     don't appear as both a generator and a regular model.
    //   - Include the emitted virtual ModelFile entries produced above.
    let mut models: Vec<ModelFile> = sql_models
        .into_iter()
        .filter(|m| !m.name.ends_with(".gen") && !m.path.to_string_lossy().contains(".gen."))
        .collect();
    models.extend(emitted_model_files);

    // Collect test-model → target-model mapping before filtering test models out,
    // so each catalog model page can list the tests that exercise it.
    let mut test_targets: HashMap<String, Vec<TestRef>> = HashMap::new();
    for model in &models {
        if model.is_test() {
            if let Some(tc) = model.test_config() {
                let target = tc.model.clone();
                let test_ref = TestRef {
                    name: model.name.clone(),
                    path: model.path.display().to_string(),
                };
                test_targets.entry(target).or_default().push(test_ref);
            }
        }
    }

    // Filter out test models
    models.retain(|m| !m.is_test());

    let python_files = discovery
        .discover_python_files()
        .with_context(|| "Failed to scan for Python models")?;

    if !python_files.is_empty() {
        let python_models = discover_python_models(
            &python_files,
            &models,
            &config,
            &project_dir,
            config.python.as_deref(),
        )
        .with_context(|| "Failed to discover Python models")?;
        models.extend(python_models);
    }

    let graph = DependencyGraph::build(models.clone(), sources.as_ref())
        .with_context(|| "Failed to build dependency graph")?;

    graph
        .validate()
        .with_context(|| "Dependency validation failed")?;

    // Apply --select filters if provided
    let (graph, origins) = if !args.select.is_empty() {
        let selectors: Vec<_> = args
            .select
            .iter()
            .map(|s| parse_selector(s))
            .collect::<Result<_, _>>()
            .with_context(|| "Failed to parse selector")?;
        let selected = graph.select_models(&selectors, &config)?;
        let filtered_models: Vec<_> = models
            .into_iter()
            .filter(|m| {
                let cp = m.canonical_path();
                selected.contains(&cp) || selected.contains(&m.name)
            })
            .collect();
        let filtered_graph = DependencyGraph::build(filtered_models, sources.as_ref())
            .with_context(|| "Failed to build filtered dependency graph")?;
        (filtered_graph, origins)
    } else {
        (graph, origins)
    };

    // Initialize Salsa DB for type inference (uses the final post-filter model set).
    let db = init_db(
        &project_dir,
        &graph
            .iter_models()
            .map(|(_, m)| m.clone())
            .collect::<Vec<_>>(),
    );

    let catalog = smelt_cli::docs::build_catalog(&graph, &config, &db, &origins, &test_targets)?;

    let output_dir = args
        .output
        .unwrap_or_else(|| project_dir.join("target").join("docs"));

    match args.format.as_str() {
        "json" => {
            smelt_cli::docs_render::render_json(&catalog, &output_dir)?;
            println!("Wrote {}/catalog.json", output_dir.display());
        }
        "markdown" | "md" => {
            smelt_cli::docs_render::render_markdown(&catalog, &output_dir)?;
            println!(
                "Wrote {} model pages to {}/",
                catalog.models.len(),
                output_dir.display()
            );
        }
        other => {
            anyhow::bail!("Unknown format '{}'. Supported: markdown, json", other);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_docs_include_quickstart() {
        let topics = list_topics();
        assert!(
            topics.iter().any(|t| t == "getting-started/quickstart"),
            "expected getting-started/quickstart in {:?}",
            topics
        );
    }

    #[test]
    fn show_returns_non_empty_markdown() {
        let file = lookup_topic("getting-started/quickstart").expect("quickstart present");
        let body = file.contents_utf8().expect("utf-8 body");
        assert!(
            body.contains("Quickstart"),
            "body should mention Quickstart"
        );
        assert!(body.len() > 500, "body too short: {} bytes", body.len());
    }

    #[test]
    fn show_accepts_md_suffix() {
        assert!(lookup_topic("index").is_some());
        assert!(lookup_topic("index.md").is_some());
    }

    #[test]
    fn unknown_topic_returns_none() {
        assert!(lookup_topic("nonsense/does-not-exist").is_none());
    }
}
