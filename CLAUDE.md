# Claude AI Assistant Guidelines for cargo-appraiser

## Project Overview

`cargo-appraiser` is an LSP server for `Cargo.toml` files that helps developers understand the relationship between declared dependencies (desired state in `Cargo.toml`) and what `cargo` has actually resolved (actual state in `Cargo.lock`). This provides insights for keeping dependencies up-to-date, identifying version conflicts, and navigating complex workspaces.

## Key Technologies

- **Language**: Rust
- **Async Runtime**: Tokio
- **LSP Implementation**: tower-lsp
- **TOML Parsing**: taplo
- **Main Dependencies**: cargo (custom fork), semver, reqwest

## Development Guidelines

### Rust Best Practices

- Write clear, concise, and idiomatic Rust code with accurate examples
- Use async programming paradigms effectively, leveraging `tokio` for concurrency
- Prioritize modularity, clean code organization, and efficient resource management
- Use expressive variable names that convey intent (e.g., `is_ready`, `has_data`)
- Adhere to Rust's naming conventions: snake_case for variables and functions, PascalCase for types and structs
- Avoid code duplication; use functions and modules to encapsulate reusable logic
- Write code with safety, concurrency, and performance in mind, embracing Rust's ownership and type system

### Async Programming

- Use `tokio` as the async runtime for handling asynchronous tasks and I/O
- Implement async functions using `async fn` syntax
- Leverage `tokio::spawn` for task spawning and concurrency
- Use `tokio::select!` for managing multiple async tasks and cancellations
- Favor structured concurrency: prefer scoped tasks and clean cancellation paths
- Implement timeouts, retries, and backoff strategies for robust async operations

### Channels and Concurrency

- Use Rust's `tokio::sync::mpsc` for asynchronous, multi-producer, single-consumer channels
- Use `tokio::sync::broadcast` for broadcasting messages to multiple consumers
- Implement `tokio::sync::oneshot` for one-time communication between tasks
- Prefer bounded channels for backpressure; handle capacity limits gracefully
- Use `tokio::sync::Mutex` and `tokio::sync::RwLock` for shared state across tasks, avoiding deadlocks

### Error Handling and Safety

- Embrace Rust's Result and Option types for error handling
- Use `?` operator to propagate errors in async functions
- Implement custom error types using `thiserror` or `anyhow` for more descriptive errors
- Handle errors and edge cases early, returning errors where appropriate
- Use `.await` responsibly, ensuring safe points for context switching

### Testing

- Write unit tests with `tokio::test` for async tests
- Use `tokio::time::pause` for testing time-dependent code without real delays
- Implement integration tests to validate async behavior and concurrency
- Use mocks and fakes for external dependencies in tests

### Performance Optimization

- Minimize async overhead; use sync code where async is not needed
- Avoid blocking operations inside async functions; offload to dedicated blocking threads if necessary
- Use `tokio::task::yield_now` to yield control in cooperative multitasking scenarios
- Optimize data structures and algorithms for async use, reducing contention and lock duration
- Use `tokio::time::sleep` and `tokio::time::interval` for efficient time-based operations

## Project-Specific Architecture

### Module Structure

The project is organized into clear modules:

- **controller/**: Core LSP functionality (appraiser, audit, capabilities, cargo integration, code actions, completion, diagnostics, hover, etc.)
- **entity/**: Domain models (dependency, manifest, package, workspace, etc.)
- **usecase/**: Business logic (document handling, symbol trees)
- **decoration/**: UI hints and decorations (inlay hints, VSCode-specific decorations)

### Key Features to Maintain

1. **Workspace Support**: Handle cargo workspace configurations properly
2. **Dependency Analysis**: 
   - Version decorations showing dependency status
   - Hover information for versions, git dependencies, and features
   - Code actions for updating dependencies
3. **Audit Integration**: Security vulnerability detection via cargo-audit
4. **LSP Features**:
   - Hover tooltips
   - Code actions
   - Go to definition for workspace dependencies
   - Inlay hints/decorations

### Configuration Support

The LSP supports configuration through:
- VSCode-specific settings (decoration colors, extra environment variables, server path)
- LSP initialization options (decoration formatter, audit settings)

### Important Implementation Details

1. **Async LSP Server**: All LSP methods should be async and non-blocking
2. **Cargo Integration**: Uses a custom fork of cargo for dependency resolution
3. **Caching**: Consider caching dependency information to avoid repeated cargo calls
4. **Error Recovery**: The LSP should gracefully handle malformed TOML files
5. **Platform Support**: Ensure cross-platform compatibility (Windows, macOS, Linux)

## Development Workflow

1. **Dependencies**: Always check if a dependency update affects the cargo fork integration
2. **Testing**: Test with various Cargo.toml configurations including:
   - Simple single-package projects
   - Workspace configurations
   - Git dependencies
   - Path dependencies
   - Feature configurations
3. **Editor Support**: Primary support is for VSCode, with secondary support for Zed and Vim
4. **Performance**: Monitor performance with large workspaces and many dependencies

## Common Tasks

- **Adding new LSP features**: Implement in the appropriate controller module
- **Enhancing dependency analysis**: Update entity models and appraiser logic
- **Improving decorations**: Modify decoration formatter and inlay hint generation
- **Adding new code actions**: Extend the code_action controller

## Documentation

- Keep the README.md updated with new features
- Document configuration options clearly
- Provide examples for complex features
- Update editor-specific documentation when adding new capabilities