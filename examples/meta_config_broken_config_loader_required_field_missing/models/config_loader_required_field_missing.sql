-- Intentional error: configs/incomplete.yaml is missing the `threshold` field
-- required by the schema {name: Text, region: Text, threshold: Integer}.
-- This emits ConfigLoaderRequiredFieldMissing anchored at the YAML record.
SELECT smelt.config.load_yaml('configs/incomplete.yaml', {name: Text, region: Text, threshold: Integer})
