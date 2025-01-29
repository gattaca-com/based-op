use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RpcParam {
    String(String),
    Bool(bool),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RpcRequest<'a> {
    pub jsonrpc: &'a str,
    pub method: &'a str,
    pub params: Vec<RpcParam>,
    pub id: u16,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RpcResponse<T> {
    pub jsonrpc: String,
    pub id: u16,
    pub result: T,
}
