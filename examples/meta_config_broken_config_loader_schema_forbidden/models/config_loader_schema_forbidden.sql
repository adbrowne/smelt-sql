-- Intentional error: schema is a bare scalar type (Integer), not a record,
-- List<record>, or Map<Text, record>. Scalar schemas are forbidden because
-- the loader requires a structured record shape. This emits ConfigLoaderSchemaForbidden.
SELECT smelt.config.load_yaml('configs/data.yaml', Integer)
