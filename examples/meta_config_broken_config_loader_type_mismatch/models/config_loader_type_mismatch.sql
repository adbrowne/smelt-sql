-- Intentional error: configs/wrong_type.yaml has `threshold: not_a_number`
-- but the schema declares `threshold: Integer`. A string value where an
-- Integer is expected emits ConfigLoaderTypeMismatch at the YAML value row.
SELECT smelt.config.load_yaml('configs/wrong_type.yaml', {name: Text, region: Text, threshold: Integer})
