-- Wide-reflection: list all source names tagged `audit`.
--
-- smelt.sources.with_tag('audit') returns List<SourceRef> for sources whose
-- YAML declares tags: [audit].  map projects to source names for inspection.
SELECT map(smelt.sources.with_tag('audit'), fn s => s.name)
