-- A trivial model. Its only purpose is to ensure `smelt build` runs the
-- diagnostic-parity gate, which must reject the build because the project's
-- sources/raw/events.yml carries an unrecognised column type (SourceTypeError).
SELECT 1 AS x
