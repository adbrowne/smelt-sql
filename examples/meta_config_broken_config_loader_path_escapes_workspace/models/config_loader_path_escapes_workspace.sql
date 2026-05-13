-- Intentional error: path uses `..` to escape the workspace root.
-- Loader paths must be workspace-relative (no `..`, no absolute paths,
-- no scheme prefixes). This emits ConfigLoaderPathEscapesWorkspace.
SELECT smelt.config.load_yaml('../shared/config.yaml', {name: Text, threshold: Integer})
