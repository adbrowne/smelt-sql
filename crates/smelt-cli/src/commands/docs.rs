use anyhow::{Context, Result};
use include_dir::{include_dir, Dir};
use smelt_cli::{
    discover_python_models, find_project_root, parse_selector, Config, LogicalGraph,
    ModelDiscovery, SourcesConfig,
};

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
    let seeds = smelt_core::discover_seed_infos(&project_dir, &config.paths);

    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let mut models = discovery
        .discover_models()
        .with_context(|| "Failed to discover models")?;

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

    let default_target = config
        .targets
        .keys()
        .next()
        .map(|s| s.as_str())
        .unwrap_or("dev");
    let graph = LogicalGraph::build(
        models.clone(),
        sources.as_ref(),
        &seeds,
        &config,
        default_target,
    )
    .with_context(|| "Failed to build logical graph")?;

    graph
        .validate()
        .with_context(|| "Dependency validation failed")?;

    // Apply --select filters if provided
    let graph = if !args.select.is_empty() {
        let selectors: Vec<_> = args
            .select
            .iter()
            .map(|s| parse_selector(s))
            .collect::<Result<_, _>>()
            .with_context(|| "Failed to parse selector")?;
        let selected = graph.select_models(&selectors)?;
        let filtered_models: Vec<_> = models
            .into_iter()
            .filter(|m| selected.contains(&m.name))
            .collect();
        LogicalGraph::build(
            filtered_models,
            sources.as_ref(),
            &seeds,
            &config,
            default_target,
        )
        .with_context(|| "Failed to build filtered logical graph")?
    } else {
        graph
    };

    // Initialize Salsa DB for type inference
    let db = smelt_cli::init_db(
        &project_dir,
        &graph
            .iter_models()
            .map(|(_, m)| m.clone())
            .collect::<Vec<_>>(),
    );

    let catalog = smelt_cli::docs::build_catalog(&graph, &config, &db)?;

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
