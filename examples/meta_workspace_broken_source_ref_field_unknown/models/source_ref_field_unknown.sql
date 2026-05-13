-- Intentional error: `schema` is not in the closed SourceRef field set
-- {path, name, tags, columns}. Accessing `s.schema` emits
-- SourceRefFieldUnknown at the `schema` field token.
SELECT map(smelt.sources.all(), fn s => s.schema)
