# Architecture

This document describes the high-level architecture of `cargo-appraiser`.

## Crate Structure

```
cargo-appraiser/
├── src/                    # Main LSP server
│   ├── main.rs             # Entry point, LanguageServer impl
│   ├── controller/         # LSP request handlers
│   │   ├── appraiser.rs    # Main orchestrator
│   │   ├── hover.rs        # Hover provider
│   │   ├── completion.rs   # Completion provider
│   │   ├── code_action.rs  # Code actions
│   │   ├── diagnostic.rs   # Diagnostic publisher
│   │   ├── audit.rs        # cargo-audit integration
│   │   └── ...
│   ├── entity/             # Shared types
│   ├── usecase/            # Domain logic (Document, Workspace)
│   ├── decoration/         # Version decorations
│   └── config.rs           # Configuration
│
└── crates/
    ├── toml-parser/        # Cargo.toml parsing with LSP positions
    ├── cargo-parser/       # Cargo dependency resolution
    └── audit-parser/       # cargo-audit output parsing
```

## Data Flow

```
┌──────────────────────────────────────────────────────────────────┐
│                         LSP Client                               │
└────────────────────────────┬─────────────────────────────────────┘
                             │
                             ▼
┌──────────────────────────────────────────────────────────────────┐
│                    CargoAppraiser (main.rs)                      │
│              implements LanguageServer trait                     │
└────────────────────────────┬─────────────────────────────────────┘
                             │
                             ▼
┌──────────────────────────────────────────────────────────────────┐
│                     Appraiser Controller                         │
│  - Orchestrates event handling                                   │
│  - Manages Document state                                        │
│  - Coordinates with sub-controllers                              │
└───────┬────────────────────┬────────────────────┬────────────────┘
        │                    │                    │
        ▼                    ▼                    ▼
┌───────────────┐   ┌───────────────┐   ┌───────────────┐
│  toml-parser  │   │ cargo-parser  │   │ audit-parser  │
│               │   │               │   │               │
│ Parse TOML    │   │ Resolve deps  │   │ Parse audit   │
│ with LSP pos  │   │ via Cargo     │   │ output        │
└───────────────┘   └───────────────┘   └───────────────┘
```

## Key Types

| Type | Location | Purpose |
|------|----------|---------|
| `Document` | `usecase/document.rs` | Parsed Cargo.toml with resolution |
| `TomlTree` | `toml-parser` | AST with position info |
| `ResolvedDependency` | `cargo-parser` | Resolved version info |
| `CargoError` | `entity/cargo_error.rs` | Typed cargo errors |
