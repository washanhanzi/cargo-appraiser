use super::{package::Package, profile::Profile, TomlEntry, TomlKey, TomlNode};

#[derive(Default)]
pub struct Manifest {
    package: Package,
    profile: Option<Vec<Profile>>,
}

pub fn row_id(key: Option<&TomlNode>, entry: Option<&TomlNode>) -> Option<String> {
    key.and_then(|k| k.row_id())
        .or_else(|| entry.and_then(|e| e.row_id()))
}
