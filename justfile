# Default recipe - list available commands
default:
    @just --list

# Build the project in debug mode
build:
    cargo build

alias b := build

# Build the project in release mode
release:
    cargo build --release

alias r := release

# Run all tests (builds binary first for CLI tests)
test *ARGS:
    cargo build
    cargo test --all-features {{ARGS}}

alias t := test

# Run tests with output shown
test-verbose:
    cargo test -- --nocapture

alias tv := test-verbose

# Run clippy lints
lint:
    cargo clippy --all-targets --all-features -- -D warnings

alias l := lint

# Format code
format:
    cargo fmt

alias fmt := format
alias f := format

# Check formatting without making changes
fmt-check:
    cargo fmt -- --check

alias fc := fmt-check

# Run all checks (format, lint, test)
check: fmt-check lint test

alias c := check

# Clean build artifacts
clean:
    cargo clean

# Install the binary locally
install:
    cargo install --path .

alias i := install

# Run the binary with arguments
run *ARGS:
    cargo run -- {{ARGS}}

# Watch for changes and run tests
watch-test:
    cargo watch -x test

alias wt := watch-test

# Watch for changes and run clippy
watch-lint:
    cargo watch -x clippy

alias wl := watch-lint

# Run all CI checks (test, lint, format check)
ci: test lint fmt-check

# Run tests with coverage (generates lcov.info)
test-coverage:
    #!/usr/bin/env bash
    set -euo pipefail
    # Set up llvm-cov environment and build the binary first
    source <(cargo llvm-cov show-env --export-prefix)
    cargo build --all-features
    cargo llvm-cov --no-clean --all-features --lcov --output-path lcov.info

alias tc := test-coverage

# Generate HTML coverage report
coverage-html:
    cargo llvm-cov nextest --all-features --html --output-dir coverage

# Open coverage report in browser
coverage-report: coverage-html
    open coverage/html/index.html || xdg-open coverage/html/index.html 2>/dev/null || echo "Open coverage/html/index.html manually"

# Run security audit
audit:
    cargo audit
    cargo deny check

alias a := audit

# Build documentation (fails on warnings)
docs:
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features

alias d := docs
