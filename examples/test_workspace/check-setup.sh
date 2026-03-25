#!/bin/bash
# Pre-flight check for testing sqt VSCode extension

set -e

echo "🔍 sqt VSCode Extension - Pre-flight Check"
echo "=========================================="
echo ""

# Check 1: Workspace structure
echo "✓ Checking workspace structure..."
if [ ! -d "models" ]; then
    echo "  ❌ ERROR: models/ directory not found"
    exit 1
fi
echo "  ✅ models/ directory exists"

# Check 2: Model files
echo ""
echo "✓ Checking model files..."
MODELS=(raw_events.sql user_sessions.sql user_stats.sql)
for model in "${MODELS[@]}"; do
    if [ ! -f "models/$model" ]; then
        echo "  ❌ ERROR: models/$model not found"
        exit 1
    fi
    echo "  ✅ models/$model"
done

# Check 3: Cargo available
echo ""
echo "✓ Checking Cargo (Rust toolchain)..."
if ! command -v cargo &> /dev/null; then
    echo "  ❌ ERROR: cargo not found in PATH"
    echo "     Install Rust from: https://rustup.rs/"
    exit 1
fi
CARGO_VERSION=$(cargo --version)
echo "  ✅ $CARGO_VERSION"

# Check 4: smelt-lsp builds
echo ""
echo "✓ Checking smelt-lsp compiles..."
cd ..
if ! cargo build -p smelt-lsp 2>&1 | grep -q "Finished"; then
    echo "  ❌ ERROR: smelt-lsp failed to build"
    exit 1
fi
echo "  ✅ smelt-lsp builds successfully"

# Check 5: VSCode extension structure
echo ""
echo "✓ Checking VSCode extension..."
if [ ! -d "editors/vscode" ]; then
    echo "  ❌ ERROR: editors/vscode directory not found"
    exit 1
fi
echo "  ✅ editors/vscode/ exists"

if [ ! -f "editors/vscode/package.json" ]; then
    echo "  ❌ ERROR: editors/vscode/package.json not found"
    exit 1
fi
echo "  ✅ package.json exists"

if [ ! -d "editors/vscode/node_modules" ]; then
    echo "  ⚠️  WARNING: node_modules not found (run: cd editors/vscode && npm install)"
else
    echo "  ✅ node_modules/ exists"
fi

if [ ! -f "editors/vscode/out/extension.js" ]; then
    echo "  ⚠️  WARNING: extension not compiled (run: cd editors/vscode && npm run compile)"
else
    echo "  ✅ extension.js compiled"
fi

# Check 6: VSCode available
echo ""
echo "✓ Checking VSCode..."
if ! command -v code &> /dev/null; then
    echo "  ⚠️  WARNING: 'code' command not found in PATH"
    echo "     Install VSCode CLI from: View → Command Palette → 'Shell Command: Install code command'"
else
    echo "  ✅ VSCode CLI available"
fi

# Summary
echo ""
echo "=========================================="
echo "✅ Pre-flight check complete!"
echo ""
echo "📋 Next Steps:"
echo "1. Open extension in VSCode:"
echo "   code editors/vscode"
echo ""
echo "2. Press F5 to launch Extension Development Host"
echo ""
echo "3. In the new window, open test workspace:"
echo "   code examples/test_workspace"
echo ""
echo "4. Follow testing guide:"
echo "   cat examples/test_workspace/TESTING.md"
echo ""
