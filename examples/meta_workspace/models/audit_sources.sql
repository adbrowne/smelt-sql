-- Wide-reflection: list all source names tagged `audit`.
--
-- smelt.sources.with_tag('audit') returns List<SourceRef> for sources whose
-- YAML declares tags: [audit].  map projects to source names; the spread
-- consumes that List<Text> into one SELECT item per name (a bare list result
-- would be MetaListInScalarPosition — every list must be consumed).
SELECT ...map(smelt.sources.with_tag('audit'), fn s => s.name)
