# audit-parser

Run `cargo audit` and provide fast indexed lookups, designed for IDE tooling.

## Features

- **Cargo audit invocation**: Runs `cargo audit` and parses text output
- **O(1) lookups**: Query audit issues by (crate_name, version) or by name only
- **Issue types**: Supports vulnerabilities and warnings (unmaintained, unsound, yanked)

## Usage

```rust
use audit_parser::AuditIndex;
use std::path::Path;

// Run cargo audit and parse results
let index = AuditIndex::audit(
    Path::new("/path/to/Cargo.lock"),
    &["my-workspace-member"],
    "cargo",
).await?;

// O(1) lookup by (name, version)
if let Some(issues) = index.get("serde", "1.0.0") {
    for issue in issues {
        println!("{}: {}", issue.id, issue.title);
    }
}

// O(1) lookup by name only
if index.has_issues_by_name("tokio") {
    println!("tokio has security issues!");
}
```

## Complexity

| Operation | Complexity |
|-----------|------------|
| `audit()` | O(n) + process I/O |
| `get(name, version)` | O(1) |
| `get_by_name(name)` | O(1) |
| `has_issues()` | O(1) |
