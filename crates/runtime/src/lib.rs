mod llama_adapter;
mod process_manager;
mod runtime;
pub mod template_engine;

#[cfg(test)]
mod pollution_tests;

#[cfg(test)]
mod tests;

pub use llama_adapter::LlamaAdapter;
pub use runtime::{ModelRuntime, RuntimeHandle};
pub use template_engine::{CleanedResponse, StreamChunkResult, TemplateEngine};

use async_trait::async_trait;
use chatsafe_common::{GenerationParams, Message, Result, StreamFrame};
use futures::Stream;
use std::pin::Pin;
use std::sync::Arc;
use std::time::SystemTime;

/// Handle to a loaded model
#[derive(Debug, Clone, PartialEq)]
pub struct ModelHandle {
    pub model_id: Arc<str>,
    pub loaded_at: SystemTime,
    pub context_size: usize,
}

/// Health status for runtime
#[derive(Debug, Clone)]
pub struct RuntimeHealth {
    pub is_healthy: bool,
    pub model_loaded: Option<ModelHandle>,
    pub active_requests: usize,
    pub uptime_seconds: u64,
}

/// Trait for model runtime implementations
#[async_trait]
pub trait Runtime: Send + Sync {
    /// Load a model and return a handle
    async fn load(&mut self, model_id: &str) -> Result<ModelHandle>;

    /// Get current loaded model handle
    async fn get_handle(&self) -> Option<ModelHandle>;

    /// Generate completion with streaming
    async fn generate(
        &self,
        handle: &ModelHandle,
        messages: Vec<Message>,
        params: GenerationParams,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamFrame>> + Send>>>;

    /// Cancel a generation request
    async fn cancel(&self, request_id: &str) -> Result<()>;

    /// Get runtime health status
    async fn health(&self) -> Result<RuntimeHealth>;

    /// Unload current model
    async fn unload(&mut self) -> Result<()>;

    /// Shutdown runtime completely
    async fn shutdown(&mut self) -> Result<()>;
}

/// Extension trait for convenience methods
#[async_trait]
pub trait RuntimeExt: Runtime {
    /// Generate without streaming (collects all frames)
    async fn generate_blocking(
        &self,
        handle: &ModelHandle,
        messages: Vec<Message>,
        params: GenerationParams,
    ) -> Result<String> {
        use futures::StreamExt;

        let mut stream = self.generate(handle, messages, params).await?;
        let mut content = String::new();

        while let Some(frame) = stream.next().await {
            match frame? {
                StreamFrame::Delta { content: delta } => {
                    content.push_str(&delta);
                }
                StreamFrame::Error { message } => {
                    return Err(chatsafe_common::Error::RuntimeError(message));
                }
                _ => {}
            }
        }

        Ok(content)
    }
}

// Implement RuntimeExt for all Runtime implementers
impl<T: Runtime + ?Sized> RuntimeExt for T {}
