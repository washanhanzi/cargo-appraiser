# Contributing

Thank you for your interest in contributing to `cargo-appraiser`!

## Development Setup

```bash
# Clone the repository
git clone https://github.com/washanhanzi/cargo-appraiser
cd cargo-appraiser

# Build
cargo build

# Run tests
cargo test --all
```

## Running Locally

```bash
# Run with inlay hints (for Zed, Neovim)
cargo run -- --renderer inlayHint --stdio

# Run with vscode decorations
cargo run -- --renderer vscode --stdio
```

## Code Style

- Run `cargo fmt` before committing
- Run `cargo clippy` and address warnings
- Add tests for new functionality

## Project Structure

See [ARCHITECTURE.md](ARCHITECTURE.md) for crate organization.

## Pull Request Process

1. Fork the repo and create your branch from `main`
2. Add tests for new functionality
3. Ensure `cargo test --all` passes
4. Update documentation if needed
5. Submit a PR with a clear description

## Testing

```bash
# Run all tests
cargo test --all

# Run main crate tests only
cargo test -p cargo-appraiser

# Run with logging
RUST_LOG=debug cargo test
```
