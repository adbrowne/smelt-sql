-- Intentional error: `not_declared_var` is not present in this workspace's vars: block.
-- smelt.config.var('not_declared_var') references a key that doesn't exist.
-- Emits: ConfigVarNotFound
SELECT smelt.config.var('not_declared_var')
