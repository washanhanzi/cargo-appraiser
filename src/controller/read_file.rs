use serde::{Deserialize, Serialize};
use tower_lsp::lsp_types::{self, Uri};

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

//TODO i need a vfs
impl lsp_types::request::Request for ReadFile {
    type Params = ReadFileParam;
    type Result = ReadFileResponse;
    const METHOD: &'static str = "textDocument/readFile";
}
