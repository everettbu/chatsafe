use thiserror::Error;
use serde::Serialize;

/// Common error type for ChatSafe with clear taxonomy
#[derive(Error, Debug)]
pub enum Error {
    /// Client request errors (4xx)
    #[error("Bad request: {0}")]
    BadRequest(String),
    
    #[error("Model not found: {0}")]
    ModelNotFound(String),
    
    #[error("Invalid model: {0}")]
    InvalidModel(String),
    
    #[error("Request validation failed: {0}")]
    ValidationFailed(String),
    
    #[error("Rate limit exceeded")]
    RateLimitExceeded,
    
    /// Service availability errors (5xx)
    #[error("Service unavailable: {0}")]
    ServiceUnavailable(String),
    
    #[error("Model loading failed: {0}")]
    ModelLoadFailed(String),
    
    #[error("Runtime not ready")]
    RuntimeNotReady,
    
    /// Timeout and cancellation errors
    #[error("Request timeout after {0} seconds")]
    Timeout(u64),
    
    #[error("Request cancelled: {0}")]
    Cancelled(String),
    
    #[error("Generation cancelled by user")]
    UserCancelled,
    
    /// Internal errors
    #[error("Internal error: {0}")]
    Internal(String),
    
    #[error("Runtime error: {0}")]
    RuntimeError(String),
    
    #[error("Configuration error: {0}")]
    ConfigError(String),
    
    /// IO and serialization errors
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    
    /// Generic anyhow error for flexibility
    #[error("Error: {0}")]
    Anyhow(#[from] anyhow::Error),
}

impl Error {
    /// Get HTTP status code for the error
    pub fn status_code(&self) -> u16 {
        match self {
            // 4xx Client Errors
            Error::BadRequest(_) => 400,
            Error::ValidationFailed(_) => 400,
            Error::ModelNotFound(_) => 404,
            Error::InvalidModel(_) => 400,
            Error::RateLimitExceeded => 429,
            
            // 5xx Server Errors
            Error::ServiceUnavailable(_) => 503,
            Error::ModelLoadFailed(_) => 503,
            Error::RuntimeNotReady => 503,
            
            // Timeout/Cancellation
            Error::Timeout(_) => 408,
            Error::Cancelled(_) => 499,
            Error::UserCancelled => 499,
            
            // Internal Errors
            Error::Internal(_) => 500,
            Error::RuntimeError(_) => 500,
            Error::ConfigError(_) => 500,
            Error::Io(_) => 500,
            Error::Serialization(_) => 500,
            Error::Anyhow(_) => 500,
        }
    }
    
    /// Get error type for metrics/logging
    pub fn error_type(&self) -> &'static str {
        match self {
            Error::BadRequest(_) => "bad_request",
            Error::ValidationFailed(_) => "validation_failed",
            Error::ModelNotFound(_) => "model_not_found",
            Error::InvalidModel(_) => "invalid_model",
            Error::RateLimitExceeded => "rate_limit",
            Error::ServiceUnavailable(_) => "service_unavailable",
            Error::ModelLoadFailed(_) => "model_load_failed",
            Error::RuntimeNotReady => "runtime_not_ready",
            Error::Timeout(_) => "timeout",
            Error::Cancelled(_) => "cancelled",
            Error::UserCancelled => "user_cancelled",
            Error::Internal(_) => "internal",
            Error::RuntimeError(_) => "runtime_error",
            Error::ConfigError(_) => "config_error",
            Error::Io(_) => "io_error",
            Error::Serialization(_) => "serialization_error",
            Error::Anyhow(_) => "unknown",
        }
    }
    
    /// Check if error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Error::ServiceUnavailable(_) | 
            Error::RuntimeNotReady | 
            Error::Timeout(_) |
            Error::Io(_)
        )
    }
}

/// Error response for HTTP API
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: ErrorDetail,
}

#[derive(Debug, Serialize)]
pub struct ErrorDetail {
    pub message: String,
    pub r#type: String,
    pub code: u16,
}

impl From<&Error> for ErrorResponse {
    fn from(err: &Error) -> Self {
        ErrorResponse {
            error: ErrorDetail {
                message: err.to_string(),
                r#type: err.error_type().to_string(),
                code: err.status_code(),
            },
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;