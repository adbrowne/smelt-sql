-- Intentional error: the referenced YAML file does not exist in the workspace.
-- smelt.config.load_yaml validates that the target file exists at the resolved
-- workspace-relative path. This emits ConfigLoaderFileNotFound.
SELECT smelt.config.load_yaml('configs/nonexistent.yaml', {name: Text, threshold: Integer})
