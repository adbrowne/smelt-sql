-- Intentional error: `bogus` is not in the closed accessor set {with_tag, all}.
-- smelt.models.bogus() emits WideReflectionUnknownAccessor at the `bogus` token.
SELECT smelt.models.bogus()
