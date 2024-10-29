use super::Value;

#[derive(Default, Debug, Clone)]
pub struct Profile {
    name: Option<Value<String>>,
}
