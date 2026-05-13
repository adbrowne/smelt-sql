-- Intentional error: configs/list_data.yaml is a YAML sequence (list at root),
-- but the schema is a record type {name: Text, region: Text, threshold: Integer}
-- which expects a mapping at the root. This emits ConfigLoaderRootShapeMismatch.
SELECT smelt.config.load_yaml('configs/list_data.yaml', {name: Text, region: Text, threshold: Integer})
