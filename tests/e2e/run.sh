#!/bin/bash
# E2E Test Runner for kto
#
# Usage:
#   ./run.sh              Run all tests
#   ./run.sh --verbose    Show details for failed tests
#   ./run.sh -s price     Run only scenarios containing "price"

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Check dependencies
if ! python3 -c "import flask" 2>/dev/null; then
    echo "Installing test dependencies..."
    pip3 install -q flask requests
fi

# Build kto first
echo "Building kto..."
cd ../..
cargo build --quiet
cd "$SCRIPT_DIR"

# Run the test suite
echo ""
python3 run_suite.py "$@"
