-- Happy path: declare a smelt.record, load a YAML file as Map<Text, Tenant>,
-- then consume it via .keys() → reduce → a single scalar.
--
-- No FROM clause → check_type_diagnostics short-circuits; only the loader
-- path/schema/content checks from file_diagnostics run.
smelt.record Tenant = { plan: Text, threshold: Integer }

SELECT reduce(
  map(
    smelt.config.load_yaml('configs/tenants.yaml', Map<Text, {plan: Text, threshold: Integer}>).keys(),
    fn k => k
  ),
  concat_with(', ')
) AS tenant_keys
