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
