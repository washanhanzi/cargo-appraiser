use std::collections::HashMap;

use super::{TomlEntry, TomlKey};

#[derive(Debug, Clone)]
pub struct SymbolTree {
    pub entries: HashMap<String, TomlEntry>,
    pub keys: HashMap<String, TomlKey>,
}
