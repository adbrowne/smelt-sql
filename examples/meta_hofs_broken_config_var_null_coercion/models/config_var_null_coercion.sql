-- Intentional warning: smelt.config.var('region') resolves to a YAML null (~)
-- which is coerced to an empty string. This triggers a ConfigVarNullCoercion warning.
-- Emits: ConfigVarNullCoercion
SELECT smelt.config.var('region')
