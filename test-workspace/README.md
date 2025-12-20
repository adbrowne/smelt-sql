# Test Workspace for sqt LSP

This directory contains test SQL models to demonstrate the LSP features.

## Models

- **raw_events.sql**: Base model with no dependencies
- **user_sessions.sql**: References raw_events ✓
- **user_stats.sql**: References user_sessions ✓
- **broken_model.sql**: References nonexistent_model ✗ (should show error)

## Testing the LSP

1. Open this workspace in an editor with LSP support
2. Configure the editor to use the sqt LSP server: `cargo run -p sqt-lsp`
3. Open one of the SQL files
4. You should see:
   - Error diagnostic in broken_model.sql
   - Go-to-definition works on ref('raw_events') in user_sessions.sql
   - Go-to-definition works on ref('user_sessions') in user_stats.sql

## Features Implemented

✅ Diagnostics (undefined refs)
✅ Go-to-definition for {{ ref() }}
🚧 Completions (future)
🚧 Hover information (future)
