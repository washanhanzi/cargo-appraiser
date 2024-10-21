use super::{package::Package, profile::Profile};

#[derive(Default)]
pub struct Manifest {
    package: Package,
    profile: Option<Vec<Profile>>,
}
