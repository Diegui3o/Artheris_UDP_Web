pub mod http_server;
mod ilp;
pub mod questdb;
mod stats;
pub mod server;

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
pub use questdb::OptionalDb;
pub use server::{start_ws_server, WsContext, AvailableFieldIndex};
pub use http_server::start_http_server;