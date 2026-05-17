-- A ModelDef literal in a regular (non-generator) model file emits
-- ModelDefOutsideGeneratorFile.
SELECT ModelDef { name: 'foo', body: SELECT 1 } AS result
