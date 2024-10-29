use tower_lsp::lsp_types::Range;

use super::Value;

#[derive(Default, Debug, Clone)]
pub struct Workspace {
    pub members: Members,
}

#[derive(Default, Debug, Clone)]
struct Members {
    pub id: String,
    pub text: String,
    pub range: Range,
    pub members: Vec<Value<String>>,
}
