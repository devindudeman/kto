.PHONY: build release install test test-e2e test-all clean run ui

# Default target
all: build

# Debug build
build:
	cargo build --features tui

# Release build
release:
	cargo build --release --features tui

# Build and install to ~/.local/bin
install: release
	@pkill -x kto 2>/dev/null || true
	@sleep 0.2
	cp target/release/kto ~/.local/bin/kto
	@echo "Installed kto v$$(grep '^version' Cargo.toml | cut -d'"' -f2) to ~/.local/bin/kto"

# Run unit tests
test:
	cargo test

# Run E2E change detection tests
test-e2e: build
	python3 tests/e2e/run_suite.py

# Run all tests (unit + E2E)
test-all: test test-e2e

# Clean build artifacts
clean:
	cargo clean

# Run debug build directly
run:
	cargo run --features tui -- ui

# Run release build directly (without installing)
ui:
	cargo run --release --features tui -- ui
