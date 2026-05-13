-- Intentional error: bare empty list literal [] in a SELECT item with no
-- inferable target sort.  The type checker cannot determine the element type.
-- Emits: MetaListEmptyTypeUnknown
--
-- Note: ...[] (spread of an empty list) is silently elided per spec rule 7.
-- The bare [] used here as a SELECT item (without spread) triggers this
-- diagnostic.
SELECT
    id,
    []
FROM smelt.sources.raw.users
