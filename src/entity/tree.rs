use std::collections::HashMap;

use super::TomlNode;

#[derive(Debug, Clone)]
pub struct SymbolTree {
    pub entries: HashMap<String, TomlNode>,
    pub keys: HashMap<String, TomlNode>,
}
