use super::Value;

//Package is a semantic representation of cargo.toml's package table
#[derive(Default)]
pub struct Package {
    name: Option<Value<String>>,
    version: Option<Value<String>>,
    edition: Option<Value<String>>,
    authors: Option<Value<String>>,
    description: Option<Value<String>>,
    license: Option<Value<String>>,
    repository: Option<Value<String>>,
    homepage: Option<Value<String>>,
    documentation: Option<Value<String>>,
    readme: Option<Value<String>>,
    workspace: Option<Value<String>>,
}
