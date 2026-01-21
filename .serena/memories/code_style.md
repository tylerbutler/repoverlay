# repoverlay - Code Style and Conventions

## Rust Conventions
- Rust 2024 edition
- Standard Rust naming conventions (snake_case for functions/variables, CamelCase for types)
- Use `Result<T>` with anyhow for error handling
- Functions use `bail!` macro for early returns with errors
- Context added to errors with `.context()` or `.with_context()`

## Module Organization
- Each module has a doc comment at the top
- Public items have doc comments
- Tests are organized in `mod tests` at the bottom of files
- Test modules are further organized by functionality (e.g., `mod apply`, `mod remove`)

## Error Handling
- Use `anyhow::Result` for functions that can fail
- Use `bail!` for early error returns
- Add context to errors for better debugging

## Testing Patterns
- Test helpers like `create_test_repo()` and `create_test_overlay()` in main.rs
- Integration tests use tempfile for temporary directories
- CLI tests use assert_cmd with predicates
