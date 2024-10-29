use super::{
    package::Package, profile::Profile, workspace::Workspace, TomlEntry, TomlKey, TomlNode,
};

#[derive(Default, Debug, Clone)]
pub struct Manifest {
    pub package: Package,
    pub profile: Option<Vec<Profile>>,
    pub workspace: Option<Workspace>,
}
