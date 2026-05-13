-- Intentional error: smelt.config.load_toml is reserved (not yet supported).
-- Only smelt.config.load_yaml and smelt.config.load_json are available in v1.
-- This emits ConfigLoaderTomlNotYetSupported at the call expression.
SELECT smelt.config.load_toml('configs/settings.toml', {name: Text, threshold: Integer})
