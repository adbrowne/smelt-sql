# Broken Examples

Intentionally broken models for testing error handling, diagnostics, and LSP error reporting.

## Models

| Model | Error Type |
|-------|-----------|
| `broken_model.sql` | Undefined reference (`smelt.ref('nonexistent_model')`) |
| `parse_error.sql` | Missing FROM clause |
| `circular_ref.sql` | Self-referencing circular dependency |
| `bad_source.sql` | Non-existent source reference |
| `multiple_errors.sql` | Undefined ref + trailing syntax error |
