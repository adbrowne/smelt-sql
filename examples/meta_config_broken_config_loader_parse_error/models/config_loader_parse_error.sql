-- Intentional error: configs/bad.yaml is not valid YAML (malformed indentation).
-- smelt.config.load_yaml validates the file format before schema checking.
-- This emits ConfigLoaderParseError anchored at the loader call site.
SELECT smelt.config.load_yaml('configs/bad.yaml', {name: Text, threshold: Integer})
