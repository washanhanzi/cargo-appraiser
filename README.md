# cargo-appraiser

# Features

- Workspace
  - hover on members will show the list of members
- Dependencies
  - hover on version will show the available versions
  - hover on git dependency will show the git reference and commit
  - hover on `features` will show available features, hover on a feature name
    will show its values
  - code action on dependency's `version`
  - `cargo update` code action on dependency's `version` and `workspace`
  - goto definition on workspace dependency

# Config

- vscode specific config

```jsonc
{
  //vscode decoration color config, for example `cargo-appraiser.decorationColor.light: {latest: "#006400"}`
  "decorationColor": {
    //the default for light and highContrastLight
    "light": {
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
    "dark": {
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
    "highContrast": {
      //same as dark
    },
    "highContrastLight": {
      //same as light
    }
  }
}
```

- lsp initialization options

to apply these config changes, you need to restart the lsp

```jsonc
{
  // use cargo-appraiser.decorationFormatter in vscode settings
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
  // git: the dependency is a git dependency, support {{ref}}, {{commit}}
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
  }
}
```

# Supported Editors

- [VS Code](https://marketplace.visualstudio.com/items?itemName=washan.cargo-appraiser)
- [Zed](https://github.com/washanhanzi/zed-cargo-appraiser)
- [Vim](https://github.com/washanhanzi/cargo-appraiser.nvim)

# Thanks to

- [taplo](https://github.com/tamasfe/taplo)
- [rust analyzer](https://github.com/rust-lang/rust-analyzer)
- [cargo](https://github.com/rust-lang/cargo)
- [rustsec](https://github.com/rustsec/rustsec)
- [cargotom](https://github.com/frederik-uni/cargotom)
