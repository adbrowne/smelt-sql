-- Happy path: declare a smelt.record, load a YAML file as List<Cohort>,
-- then consume it via map → reduce → a single scalar.
--
-- The per-target overlay `configs/cohorts.prod.yaml` raises thresholds for prod
-- (list-replace semantics per the spec).
--
-- No FROM clause → check_type_diagnostics short-circuits; only the loader
-- path/schema/content checks from file_diagnostics run.
smelt.record Cohort = { name: Text, region: Text, threshold: Integer }

SELECT reduce(
  map(
    smelt.config.load_yaml('configs/cohorts.yaml', List<{name: Text, region: Text, threshold: Integer}>),
    fn c => c.name
  ),
  concat_with(', ')
) AS cohort_names
