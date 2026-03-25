# Testing the smelt VSCode Extension

This guide walks through testing the smelt VSCode extension with the examples/test_workspace.

## Test Environment

**Models in examples/test_workspace/models/**:
- `raw_events.sql` - Base model (no dependencies)
- `user_sessions.sql` - Valid reference to `{{ ref('raw_events') }}`
- `user_stats.sql` - Valid reference to `{{ ref('raw_events') }}`
- See `examples/broken/` for intentionally broken models

## Testing Methods

### Method 1: Extension Development Host (Recommended)

This launches a separate VSCode window with the extension loaded.

**Steps:**
1. Open the extension source in VSCode:
   ```bash
   code editors/vscode
   ```

2. Press **F5** to launch the Extension Development Host
   - This opens a new VSCode window with "[Extension Development Host]" in the title
   - The extension is loaded and active in this window

3. In the Extension Development Host window, open the test workspace:
   - File → Open Folder
   - Select: `examples/test_workspace`
   - Or use CLI: `code examples/test_workspace`

4. Wait for the extension to activate (check Output panel for "sqt Language Server")

5. Run the tests below

### Method 2: Direct Workspace Testing

Open the test workspace directly (requires extension to be installed).

**Steps:**
1. Open the test workspace in VSCode:
   ```bash
   code examples/test_workspace/test_workspace.code-workspace
   ```

2. The extension should auto-activate when it detects `models/*.sql`

3. Check the Output panel: View → Output → "sqt Language Server"

4. Run the tests below

## Tests to Perform

### ✅ Test 1: Extension Activation

**Expected:**
- Extension activates when workspace contains `models/*.sql`
- Info message: "sqt language server is running"
- Output panel shows LSP server starting

**Verify:**
1. Open Output panel: View → Output
2. Select "sqt Language Server" from dropdown
3. Should see: "sqt extension activating..." and server logs

**Troubleshooting:**
- If no activation, check for `models/` directory
- Ensure workspace folder is open (not just files)
- Check for errors in Output panel

---

### ✅ Test 2: Diagnostics for Undefined Reference

**Expected:**
- `broken_model.sql` shows error diagnostic for undefined `nonexistent_model`
- Error message: "Undefined model reference: nonexistent_model"

**Verify:**
1. Open `models/broken_model.sql`
2. Look for red squiggly underline on line with `{{ ref('nonexistent_model') }}`
3. Hover over the error or check Problems panel (Cmd+Shift+M)

**Current Limitation:**
- Error shows on line 0 (position tracking not yet implemented)
- Will be fixed when Rowan parser is added

**Troubleshooting:**
- Save the file to trigger re-analysis
- Check Output panel for LSP activity
- Verify LSP server is running

---

### ✅ Test 3: Valid References (No Errors)

**Expected:**
- `user_sessions.sql` shows NO errors
- `user_stats.sql` shows NO errors
- Both reference existing `raw_events` model

**Verify:**
1. Open `models/user_sessions.sql`
2. No errors on `{{ ref('raw_events') }}` line
3. Open `models/user_stats.sql`
4. No errors on `{{ ref('raw_events') }}` line

---

### ✅ Test 4: Go-to-Definition

**Expected:**
- Cmd+Click (Mac) or Ctrl+Click (Windows/Linux) on `'raw_events'` jumps to `raw_events.sql`

**Verify:**
1. Open `models/user_sessions.sql`
2. Find the line: `FROM {{ ref('raw_events') }}`
3. Cmd+Click on `'raw_events'` (the string inside ref())
4. Should jump to `models/raw_events.sql`

**Troubleshooting:**
- Ensure cursor is on the model name string
- Check Output panel for "Received goto definition request"
- Verify `raw_events.sql` exists in models/ directory

---

### ✅ Test 5: Syntax Highlighting

**Expected:**
- SQL keywords highlighted (SELECT, FROM, WHERE, etc.)
- Template expressions `{{ }}` highlighted differently
- Comments highlighted in gray/green

**Verify:**
1. Open any `.sql` file
2. Check that `SELECT`, `FROM`, `GROUP BY` are highlighted
3. Check that `{{ ref('...') }}` has distinct highlighting
4. Check that `--` comments are styled differently

---

### ✅ Test 6: Incremental Updates

**Expected:**
- Changes to files trigger re-analysis
- Diagnostics update in real-time
- Fast response (<50ms for small projects)

**Verify:**
1. Open `models/broken_model.sql`
2. Change `nonexistent_model` to `raw_events`
3. Save the file
4. Error should disappear immediately
5. Change back to `nonexistent_model` and save
6. Error should reappear

**Salsa Incremental Behavior:**
- Only changed file is re-analyzed (not entire project)
- Dependencies automatically tracked
- Check Output panel for analysis timing

---

### ✅ Test 7: Multiple File Changes

**Expected:**
- Creating new model makes it available for references
- Deleting model causes errors in dependent files

**Verify:**
1. Create new file `models/test_model.sql` with content:
   ```sql
   SELECT * FROM source.test
   ```
2. Open `models/user_sessions.sql`
3. Change `{{ ref('raw_events') }}` to `{{ ref('test_model') }}`
4. Save - should have no errors
5. Delete `models/test_model.sql`
6. Save `user_sessions.sql` again
7. Should show error about undefined `test_model`

---

## Debugging Tips

### Check LSP Server Logs

**Output Panel:**
- View → Output
- Select "sqt Language Server"
- Shows all LSP server communication

**Verbose Tracing:**
- Settings → Extensions → sqt
- Set "sqt.trace.server" to "verbose"
- Shows all LSP protocol messages

### Common Issues

**Extension not activating:**
- ✓ Workspace folder must be open (not just files)
- ✓ Must have `models/` directory with `.sql` files
- ✓ Check for activation errors in Output panel

**LSP server not starting:**
- ✓ Cargo must be in PATH
- ✓ Run `cargo build -p smelt-lsp` to verify it compiles
- ✓ Check Output panel for server startup errors

**Diagnostics not showing:**
- ✓ Save the file to trigger analysis
- ✓ Check file is in `models/` directory
- ✓ Verify LSP server is running (Output panel)

**Go-to-definition not working:**
- ✓ Referenced model must exist in `models/`
- ✓ Model name must match filename (case-sensitive)
- ✓ Click directly on the model name string

## Expected Results Summary

| Test | File | Expected Result |
|------|------|----------------|
| Activation | - | Extension starts, shows info message |
| Diagnostics | `broken_model.sql` | Error on undefined reference |
| Valid refs | `user_sessions.sql`, `user_stats.sql` | No errors |
| Go-to-def | `user_sessions.sql` → `raw_events.sql` | Navigation works |
| Syntax | All `.sql` files | Keywords and templates highlighted |
| Incremental | Edit & save | Fast updates (<50ms) |
| Multi-file | Create/delete models | Diagnostics update correctly |

## Next Steps After Testing

If all tests pass:
- ✅ Extension is working correctly
- Consider testing with larger workspace (more models)
- Consider adding more complex test cases

If tests fail:
- Check Output panel for errors
- Review LSP server logs
- Verify workspace structure
- Check that Cargo/Rust is properly installed

## Performance Benchmarks

Expected performance on examples/test_workspace (4 models):

- **Cold start**: ~5-10ms (first file open)
- **File edit → diagnostics**: ~2-5ms
- **Go-to-definition**: ~1-2ms
- **Server startup**: ~500ms-2s (cargo run) or ~100ms (pre-built binary)

For better performance, build LSP server in release mode:
```bash
cargo build --release -p smelt-lsp
# Then set sqt.serverPath to: target/release/smelt-lsp
```
