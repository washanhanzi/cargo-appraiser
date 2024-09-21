use std::{
    convert::Infallible,
    fmt::{Display, Formatter},
};

use cargo::core::dependency::DepKind;
use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub enum CargoTable {
    Package,
    Lib,
    Bin,
    Example,
    Test,
    Bench,
    Dependencie(DependencyTable),
    Features,
    Workspace,
    Profile,
    Target,
    Patch,
    Replace,
    Metadata,
    Badges,
    Lints,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Default)]
pub enum DependencyTable {
    #[default]
    Dependencies,
    DevDependencies,
    BuildDependencies,
}

impl Display for DependencyTable {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                DependencyTable::Dependencies => "dependencies",
                DependencyTable::DevDependencies => "dev-dependencies",
                DependencyTable::BuildDependencies => "build-dependencies",
            }
        )
    }
}

impl From<DepKind> for DependencyTable {
    fn from(value: DepKind) -> Self {
        match value {
            DepKind::Normal => DependencyTable::Dependencies,
            DepKind::Development => DependencyTable::DevDependencies,
            DepKind::Build => DependencyTable::BuildDependencies,
        }
    }
}

impl std::str::FromStr for CargoTable {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "package" => Ok(CargoTable::Package),
            "lib" => Ok(CargoTable::Lib),
            "bin" => Ok(CargoTable::Bin),
            "example" => Ok(CargoTable::Example),
            "test" => Ok(CargoTable::Test),
            "bench" => Ok(CargoTable::Bench),
            "dependencies" => Ok(CargoTable::Dependencie(DependencyTable::Dependencies)),
            "dev-dependencies" => Ok(CargoTable::Dependencie(DependencyTable::DevDependencies)),
            "build-dependencies" => Ok(CargoTable::Dependencie(DependencyTable::BuildDependencies)),
            "features" => Ok(CargoTable::Features),
            "workspace" => Ok(CargoTable::Workspace),
            "profile" => Ok(CargoTable::Profile),
            "target" => Ok(CargoTable::Target),
            "patch" => Ok(CargoTable::Patch),
            "replace" => Ok(CargoTable::Replace),
            "metadata" => Ok(CargoTable::Metadata),
            "badges" => Ok(CargoTable::Badges),
            "lints" => Ok(CargoTable::Lints),
            _ => Ok(CargoTable::Unknown),
        }
    }
}
