use axum::{
    response::{IntoResponse, Response},
    Json,
};
use jsonrpsee_types::{ErrorCode, ErrorObject};
use reqwest::StatusCode;

pub type RpcResult<T> = std::result::Result<T, jsonrpsee_types::ErrorObjectOwned>;

#[derive(thiserror::Error, Debug)]
pub enum MuxError {
    #[error("internal error")]
    Internal,

    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("join error: {0}")]
    TokioJoin(#[from] tokio::task::JoinError),

    #[error("jsonrpsee error: {0}")]
    Jsonrpsee(#[from] jsonrpsee_types::ErrorObject<'static>),
}

impl IntoResponse for MuxError {
    fn into_response(self) -> Response {
        match self {
            // client errors
            MuxError::Internal |
            MuxError::TokioJoin(_) |
            MuxError::Serde(_) |
            MuxError::Reqwest(_) => (StatusCode::OK, Json(internal_error())),

            // JSON RPC error
            MuxError::Jsonrpsee(err) => (StatusCode::OK, Json(err)),
        }
        .into_response()
    }
}

fn internal_error() -> ErrorObject<'static> {
    ErrorObject::owned(
        ErrorCode::InternalError.code(),
        ErrorCode::InternalError.message(),
        None::<()>,
    )
}
