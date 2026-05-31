-- A trivial, analysis-clean model. It does not reference the malformed source;
-- its only job is to make the project have a selected model so `smelt build`
-- runs the diagnostic-parity gate, which must reject the build because the
-- project's `sources/raw/orders.yml` is malformed (BUG-032 / P2c).
SELECT 1 AS x
