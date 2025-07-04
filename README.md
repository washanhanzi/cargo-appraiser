# cargo-appraiser

![lsp release](https://img.shields.io/badge/dynamic/toml?url=https%3A%2F%2Fgithub.com%2Fwashanhanzi%2Fcargo-appraiser%2Fraw%2Frelease%2FCargo.toml&query=%24.package.version&prefix=v&label=version&labelColor=orange&color=%2376FF03)
![vscode extension release](https://img.shields.io/badge/dynamic/json?url=https%3A%2F%2Fgithub.com%2Fwashanhanzi%2Fcargo-appraiser%2Fraw%2Fvscode%2Frelease%2Feditor%2Fcode%2Fpackage.json&query=%24.version&prefix=vscode%2Fv&label=vscode%20extension&labelColor=blue&color=%2376FF03)


# What's this

`cargo-appraiser` is an LSP server for your `Cargo.toml` files. It helps you understand the relationship between your declared dependencies (the desired state in `Cargo.toml`) and what `cargo` has actually resolved and put into your `Cargo.lock` file (the actual state). This insight allows you to quickly see if your dependencies are up-to-date, identify potential version conflicts, and navigate complex workspaces with greater clarity.

# Features

- Workspace
  - hover on members will show the list of members
- Dependencies
  - version decorations ![CleanShot 2025-01-14 at 11 55 18@2x](https://github.com/user-attachments/assets/bad3f5ae-6242-4998-9d14-6aed0ebd9845)
  - hover on version will show the available versions ![CleanShot 2025-01-14 at 11 56 04@2x](https://github.com/user-attachments/assets/d04c73f3-9010-4ca4-b2d9-85af6afe4b59)
  - hover on git dependency will show the git reference and commit ![CleanShot 2025-01-14 at 11 56 55@2x](https://github.com/user-attachments/assets/37b70a50-27bc-4ad5-a851-ffe338682c1c)
  - hover on `features` will show available features, hover on a feature name
    will show its values ![CleanShot 2025-01-14 at 11 57 37@2x](https://github.com/user-attachments/assets/df9fcdc7-9f7f-41e7-9fde-43f08fe7d7b4) ![CleanShot 2025-01-14 at 11 58 26@2x](https://github.com/user-attachments/assets/55b1d02b-d01f-486e-81af-282a8027be4d)
  - code action on dependency's `version`  ![CleanShot 2025-01-14 at 12 00 57@2x](https://github.com/user-attachments/assets/ad4eab3c-d47c-415c-84c9-cc3253f15306)
  - `cargo update` code action on dependency's `version` and `workspace`
  - goto definition on workspace dependency
- Audit
  - Show warning and vulnerability information on dependencies.
  - This feature is enabled by default. To disable it, check the `audit.disabled` config.
  - Requires the `cargo audit` command. Install it by running: `cargo install cargo-audit --locked`.
  - For more details, see [cargo-audit](https://crates.io/crates/cargo-audit).

# Config

## VSCode specific config

```jsonc
{
  //the default for light and highContrastLight
  "cargo-appraiser.decorationColor.light": {
      "notParsed": "#808080",
      "latest": "#006400",
      "local": "#00008B",
      "notInstalled": "#808080",
      "mixedUpgradeable": "#B8860B",
      "compatibleLatest": "#B8860B",
      "nonCompatibleLatest": "#B8860B",
      "yanked": "#FF0000",
      "git": "#800080"
  },
  //the default for dark and highContrast
  "cargo-appraiser.decorationColor.dark": {
      "notParsed": "#808080",
      "latest": "#006400",
      "local": "#00008B",
      "notInstalled": "#808080",
      "mixedUpgradeable": "#FF8C00",
      "compatibleLatest": "#FF8C00",
      "nonCompatibleLatest": "#FF8C00",
      "yanked": "#FF0000",
      "git": "#800080"
  },
  "cargo-appraiser.decorationColor.highContrastLight": {
      //same as light
  },
  "cargo-appraiser.decorationColor.highContrast": {
      //same as dark
  },
  //extra environment variables to pass to the cargo-appraiser server
  "cargo-appraiser.extraEnv": {
    "RUST_LOG": "info"
  },
  //custom path to the cargo-appraiser server binary. If set, this will be used instead of downloading from GitHub.
  "cargo-appraiser.serverPath": ""
}
```

##  lsp initialization options

To apply these config, you need to restart the lsp.

### Examples

- VSCode

```jsonc
{
  "cargo-appraiser.decorationFormatter": {}, //see below
  "cargo-appraiser.audit": {} //see below
}
```

- Zed

```jsonc
{
    "lsp": {
        "cargo-appraiser": {
            "initialization_options": {
                "decorationFormatter": {}, //see below
                "audit": {} //see below
            }
        }
    }
}

```

### Default values

```jsonc
{
  /// the formatter has 7 fields:
  /// latest: the dependency has the latest version installed
  /// local: the dependency is a local path dependency
  /// not_installed: the dependency is not installed maybe because of platform mismatch
  /// loading: the dependency is loading
  /// mixed_upgradeable: the installed version has an compatible upgrade, but the latest version is not compatible with the current version requirement
  /// compatible_latest: the installed version can update to latest version
  /// noncompatible_latest: the installed version can't upate to latest version and there is no compatible upgrade
  /// yanked: the installed version is yanked
  /// git: the dependency is a git dependency, support {{ref}}, {{commit}} template strings
  ///
  /// a dependency is in `waiting` state for 2 possible reasons:
  /// 1. wait for `cargo` to run. `Cargo.toml` is not saved, so `cargo` haven't picked up the change.
  /// 2. wait for `cargo` to finish. `cargo` is running in process to resolve the dependency.
  ///
  /// each field's value may has 3 template strings:
  /// - installed: the installed version
  /// - latest_matched: the latest compatible version
  /// - latest: the latest version, the latest version may or may not be compatilbe with the version requirement
  ///
  /// the default formatter is:
  "decorationFormatter": {
    "latest": "✅ {{installed}}",
    "local": "Local",
    "not_installed": "Not installed",
    "waiting": "Waiting...",
    "mixed_upgradeable": "🚀🔒 {{installed}} -> {{latest_matched}},  {{latest}}",
    "compatible_latest": "🚀 {{installed}} -> {{latest}}",
    "noncompatible_latest": "🔒 {{installed}}, {{latest}}",
    "yanked": "❌ yanked {{installed}}, {{latest_matched}}",
    "git": "🐙 {{commit}}"
  },
  "audit":{
    "disabled": false,
    // "warning" will show warning and vulnerability
    // "vulnerability" will only show vulnerability
    "level": "warning"
  }
}
```

# Supported Editors

## [VS Code](https://marketplace.visualstudio.com/items?itemName=washan.cargo-appraiser)

VSCode is the main supported editor.

## [Zed](https://github.com/washanhanzi/zed-cargo-appraiser)

Enable `inlay_hints` in settings.

```jsonc
"inlay_hints": {
	"enabled": true
}
```

## [Vim](https://github.com/washanhanzi/cargo-appraiser.nvim)

Vim has minimal support for now.

# Thanks to

- [taplo](https://github.com/tamasfe/taplo)
- [rust analyzer](https://github.com/rust-lang/rust-analyzer)
- [cargo](https://github.com/rust-lang/cargo)
- [rustsec](https://github.com/rustsec/rustsec)
- [cargotom](https://github.com/frederik-uni/cargotom)
