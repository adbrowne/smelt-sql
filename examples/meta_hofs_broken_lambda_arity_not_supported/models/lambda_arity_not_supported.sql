-- Intentional error: multi-argument lambda `fn (a, b) => …` is reserved for Phase F.
-- Phase B detects this via a text shape check on the HOF's second argument
-- (the parser does not produce a multi-arg LAMBDA CST node yet).
-- Emits: LambdaArityNotSupported
SELECT map([1, 2, 3], fn (a, b) => a + b)
