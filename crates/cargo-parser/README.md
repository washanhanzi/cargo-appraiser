# cargo-parser

Resolve cargo dependencies and provide fast indexed lookups, designed for IDE tooling.

## Features

- **Cargo resolution**: Runs cargo's dependency resolution process
- **O(1) lookups**: Query resolved dependencies by (table, platform, name)
- **Version info**: Available versions, latest compatible, and absolute latest

## Overview

The `cargo-parser` crate complements `toml-parser`:
- **toml-parser**: Parses Cargo.toml → desired state with LSP positions
- **cargo-parser**: Resolves via cargo → actual state (installed packages, versions)

Both use `DependencyTable` from `toml-parser` as the shared key type.

## Usage

```rust
use cargo_parser::{CargoIndex, DependencyLookupKey, DependencyTable};
use std::path::Path;

// Resolve dependencies (runs cargo resolution)
let index = CargoIndex::resolve(Path::new("/path/to/Cargo.toml"))?;

// O(1) lookup by composite key
let key = DependencyLookupKey::new(DependencyTable::Dependencies, None, "serde");
if let Some(resolved) = index.get(&key) {
    // Installed package info
    if let Some(pkg) = &resolved.package {
        println!("Installed: {}", pkg.version());
    }

    // Version status
    if resolved.has_compatible_upgrade() {
        println!("Compatible upgrade available");
    }
    if resolved.has_incompatible_latest() {
        println!("Newer major version exists");
    }
}
```

## Integration with toml-parser

```rust
use toml_parser::{parse, DependencyTable};
use cargo_parser::{CargoIndex, DependencyLookupKey};

// Parse Cargo.toml for positions
let toml_content = std::fs::read_to_string("Cargo.toml")?;
let toml_tree = parse(&toml_content);

// Resolve via cargo for actual state
let cargo_index = CargoIndex::resolve(Path::new("Cargo.toml"))?;

// Map toml-parser dependency to cargo-parser result
for dep in toml_tree.dependencies() {
    let key = DependencyLookupKey::new(dep.table, dep.platform.clone(), &dep.name);
    if let Some(resolved) = cargo_index.get(&key) {
        // Now you have both:
        // - dep.range: LSP position from toml-parser
        // - resolved.package: installed version from cargo
    }
}
```

## Core Types

### DependencyLookupKey

Composite key for lookups (uses `DependencyTable` from toml-parser):

```rust
DependencyLookupKey {
    table: DependencyTable::Dependencies,  // Dependencies, DevDependencies, BuildDependencies
    platform: None,                         // or Some("cfg(windows)")
    name: "serde".to_string(),
}
```

### ResolvedDependency

Resolved dependency information from cargo:

| Field | Type | Description |
|-------|------|-------------|
| `package` | `Option<Package>` | Installed package (None if not resolved) |
| `available_versions` | `Vec<String>` | All versions from registry (descending) |
| `latest_matched_summary` | `Option<Summary>` | Latest compatible with version req |
| `latest_summary` | `Option<Summary>` | Absolute latest (may be incompatible) |

Helper methods:
- `is_installed()` - Has resolved package
- `is_latest()` - Installed is the absolute latest
- `has_compatible_upgrade()` - Newer compatible version exists
- `has_incompatible_latest()` - Newer incompatible version exists

## Complexity

| Operation | Complexity |
|-----------|------------|
| `resolve()` | O(n) + network I/O |
| `get()` | O(1) |
| `iter()` | O(n) |
