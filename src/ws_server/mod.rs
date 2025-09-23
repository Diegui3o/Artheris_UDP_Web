pub mod http_server;
mod ilp;
pub mod questdb;
pub mod server;
mod stats;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("Bad request: {0}")]
    BadRequest(String),
    #[error("Internal server error: {0}")]
    Internal(String),
    #[error("Not found: {0}")]
    NotFound(String),
}

// Re-export commonly used types
pub use http_server::start_http_server;
pub use questdb::OptionalDb;
pub use server::{AvailableFieldIndex, WsContext, start_ws_server};
