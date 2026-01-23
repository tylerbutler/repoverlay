---
name: test
description: Run the repoverlay test suite
---

Run the test suite for repoverlay:

1. Build the debug binary first (required for CLI integration tests)
2. Run `cargo test --all-features`
3. If tests fail, analyze the failure output and suggest fixes
4. For verbose output, use `cargo test -- --nocapture`

The test structure is in `src/main.rs` under `mod tests` with:
- Unit tests for `remove_section`, `state`
- Integration tests for `apply`, `remove`, `status`, `create`, `switch`
- CLI tests using `assert_cmd`
