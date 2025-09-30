use crate::{Runtime, RuntimeHealth, ModelHandle};
use async_trait::async_trait;
use chatsafe_common::{Message, GenerationParams, Result, StreamFrame};
use chatsafe_config::{ModelRegistry, AppConfig};
use futures::Stream;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Handle to interact with the runtime
#[derive(Clone)]
pub struct RuntimeHandle {
    inner: Arc<RwLock<Box<dyn Runtime>>>,
}

impl RuntimeHandle {
    /// Create a new runtime handle
    pub fn new(runtime: Box<dyn Runtime>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(runtime)),
        }
    }
    
    /// Load a model
    pub async fn load(&self, model_id: &str) -> Result<ModelHandle> {
        self.inner.write().await.load(model_id).await
    }
    
    /// Get current model handle
    pub async fn get_handle(&self) -> Option<ModelHandle> {
        self.inner.read().await.get_handle().await
    }
    
    /// Generate completion
    pub async fn generate(
        &self,
        handle: &ModelHandle,
        messages: Vec<Message>,
        params: GenerationParams,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamFrame>> + Send>>> {
        self.inner.read().await.generate(handle, messages, params).await
    }
    
    /// Cancel generation
    pub async fn cancel(&self, request_id: &str) -> Result<()> {
        self.inner.read().await.cancel(request_id).await
    }
    
    /// Get runtime health
    pub async fn health(&self) -> Result<RuntimeHealth> {
        self.inner.read().await.health().await
    }
    
    /// Unload model
    pub async fn unload(&self) -> Result<()> {
        self.inner.write().await.unload().await
    }
    
    /// Shutdown runtime
    pub async fn shutdown(&self) -> Result<()> {
        self.inner.write().await.shutdown().await
    }
}

/// Factory for creating model runtimes
pub struct ModelRuntime;

impl ModelRuntime {
    /// Create a runtime based on configuration
    pub async fn create(
        config: &AppConfig,
        registry: &ModelRegistry,
    ) -> Result<RuntimeHandle> {
        // For now, we only support llama.cpp
        let model_id = config.models.default_model.clone();
        let model_path = registry.get_model_path(&model_id)?;
        let model_config = registry.get_model(&model_id)?;
        let template = registry.get_model_template(&model_id)?;
        
        let adapter = crate::LlamaAdapter::new(
            model_path,
            model_config.clone(),
            template.clone(),
            config.runtime.clone(),
        )?;
        
        Ok(RuntimeHandle::new(Box::new(adapter)))
    }
}