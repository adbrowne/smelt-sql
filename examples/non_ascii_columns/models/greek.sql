---
materialization: view
---
-- Model with Greek-letter column aliases. Used to verify that diagnostic
-- byte-column positions are correct under both UTF-8 and UTF-16 encoding
-- negotiation.
--
-- α (U+03B1) and β (U+03B2) are each 2 UTF-8 bytes / 1 UTF-16 code unit.
SELECT
    1 AS α,
    2 AS β
