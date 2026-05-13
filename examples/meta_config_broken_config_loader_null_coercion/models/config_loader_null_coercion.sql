-- Intentional warning: configs/null_text.yaml has `name: ~` (YAML null)
-- for a field declared as Text in the schema. YAML null coerces to empty
-- string at Text fields; this emits ConfigLoaderNullCoercion (Warning severity).
SELECT smelt.config.load_yaml('configs/null_text.yaml', {name: Text, region: Text, threshold: Integer})
