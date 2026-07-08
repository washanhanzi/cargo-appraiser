use serde::{Deserialize, Serialize};
use tower_lsp_server::ls_types::{self, Uri};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadFileParam {
    pub uri: Uri,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadFileResponse {
    pub content: String,
}

pub enum ReadFile {}

//maybe i need a vfs
impl ls_types::request::Request for ReadFile {
    type Params = ReadFileParam;
    type Result = ReadFileResponse;
    const METHOD: &'static str = "textDocument/readFile";
}
