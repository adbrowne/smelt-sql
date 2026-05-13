-- Intentional error: configs/extra_field.yaml contains `extra_field` which
-- is not declared in the schema {name: Text, region: Text, threshold: Integer}.
-- This emits ConfigLoaderUnknownField anchored at the unexpected YAML key.
SELECT smelt.config.load_yaml('configs/extra_field.yaml', {name: Text, region: Text, threshold: Integer})
