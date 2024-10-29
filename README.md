# cargo-appraiser

# Features

- Workspace
  - hover on members will show the list of members
- Dependencies
  - hover on version will show the dependency versions

# Config

```jsonc
{
    "renderer": {
        // the formatter may has 3 template strings:
        // - installed: the installed version
        // - latest_matched: the latest compatible version
        // - latest: the latest version, the latest version may or may not be compatilbe with the version requirement
        //
        // a dependency is waiting for resolve for 2 possible reasons:
        // 1. wait for `cargo` to run. `Cargo.toml` is not saved, so `cargo` haven't picked up the change.
        // 2. wait for `cargo` to finish. `cargo` is running in process to resolve the dependency.
        //
        // the formatter has 7 fields:
        // latest: the dependency has the latest version installed
        // local: the dependency is a local path dependency
        // not_installed: the dependency is not installed
        // waiting: the dependency is waiting for resolve
        // mixed_upgradeable: the installed version has an compatible upgrade, and the latest version is not compatible with the current version requirement
        // compatible_latest: the installed version can update to latest version
        // noncompatible_latest: the installed version can't upate to latest version
        // yanked: the installed version is yanked
        "decorationFormatter": {
            "latest": "✅ {{installed}}",
            "local": "Local",
            "not_installed": "Not installed",
            "waiting": "Waiting...",
            "mixed_upgradeable": "🚀🔒 {{installed}} -> {{latest_matched}},  {{latest}}",
            "compatible_latest": "🚀 {{installed}} -> {{latest}}",
            "noncompatible_latest": "🔒 {{installed}}, {{latest}}",
            "yanked": "❌ yanked {{installed}}, {{latest_matched}}"
        }
    }
}
```

# Supported Editors

- [VS Code](https://marketplace.visualstudio.com/items?itemName=washan.cargo-appraiser)
- [Zed](https://github.com/washanhanzi/zed-cargo-appraiser)

# Thanks to

- [taplo](https://github.com/tamasfe/taplo)
- [rust analyzer](https://github.com/rust-lang/rust-analyzer)
- [cargo](https://github.com/rust-lang/cargo)
- [cargotom](https://github.com/frederik-uni/cargotom)
