# toml-parser

A Cargo.toml parser with LSP position support, designed for IDE tooling.

## Features

- **Position-based lookups**: O(log n) binary search to find nodes at any LSP position
- **Semantic dependency info**: O(1) lookup of dependency fields (version, git, path, features, etc.)
- **Workspace support**: Handles `[workspace.dependencies]` and target-specific dependencies

## Usage

```rust
use toml_parser::{parse, TomlTree, DependencyKey};

let toml = r#"
[dependencies]
serde = { version = "1.0", features = ["derive"] }
"#;

let result = parse(toml);

// Find node at cursor position
let node = result.tree.find_at_position(Position { line: 2, character: 5 });

// Get dependency info
let dep = result.tree.get_dependency("dependencies.serde").unwrap();
assert_eq!(dep.version().map(|v| v.text.as_str()), Some("1.0"));
```

## Module Structure

- `toml_tree` - Combined tree with position and semantic lookups
- `toml_tree::node` - TOML node types (`TomlNode`, `NodeKind`, `KeyKind`, `ValueKind`)
- `toml_tree::dependency_tree` - Dependency types (`Dependency`, `DependencyTree`)
- `toml_tree::symbol_tree` - Position-indexed tree (`SymbolTree`)
