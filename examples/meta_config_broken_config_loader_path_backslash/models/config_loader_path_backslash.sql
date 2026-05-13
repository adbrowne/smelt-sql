-- Intentional error: path uses backslash as a separator (Windows-style).
-- smelt loader paths must use forward slashes only.
-- This emits ConfigLoaderPathBackslash at the path token.
SELECT smelt.config.load_yaml('configs\tenants.yaml', {plan: Text, threshold: Integer})
