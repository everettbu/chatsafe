use async_trait::async_trait;
use chatsafe_common::{Message, Result};

/// Trait for persistent storage (future implementation)
#[async_trait]
pub trait Store: Send + Sync {
    /// Save conversation history
    async fn save_conversation(&self, id: &str, messages: &[Message]) -> Result<()>;
    
    /// Load conversation history
    async fn load_conversation(&self, id: &str) -> Result<Vec<Message>>;
    
    /// List conversation IDs
    async fn list_conversations(&self) -> Result<Vec<String>>;
}

/// No-op implementation for now
pub struct NoOpStore;

#[async_trait]
impl Store for NoOpStore {
    async fn save_conversation(&self, _id: &str, _messages: &[Message]) -> Result<()> {
        Ok(())
    }
    
    async fn load_conversation(&self, _id: &str) -> Result<Vec<Message>> {
        Ok(vec![])
    }
    
    async fn list_conversations(&self) -> Result<Vec<String>> {
        Ok(vec![])
    }
}