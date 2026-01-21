# repoverlay - Development Commands

## Task Runner (just)
- `just build` or `just b` - Build in debug mode
- `just release` or `just r` - Build in release mode
- `just test` or `just t` - Run all tests
- `just test-verbose` or `just tv` - Run tests with output shown
- `just lint` or `just l` - Run clippy lints
- `just format` or `just fmt` or `just f` - Format code
- `just fmt-check` or `just fc` - Check formatting
- `just check` or `just c` - Run all checks (format, lint, test)
- `just ci` - Run full CI suite

## Running Single Tests
```bash
cargo test <test_name>
cargo test apply::applies_single_file  # Run specific test module::test_name
```

## Running the Binary
```bash
just run <args>
cargo run -- <args>
```

## Task Completion Checklist
When completing a task, run:
1. `just fmt` - Format code
2. `just lint` - Check lints
3. `just test` - Run tests
