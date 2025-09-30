use serde::{Deserialize, Serialize};
use crate::error::{Error, Result};

// Constants for validation
const MAX_TOKENS_LIMIT: usize = 4096;
const MIN_TOKENS: usize = 1;
const TEMPERATURE_MIN: f32 = 0.0;
const TEMPERATURE_MAX: f32 = 2.0;
const TOP_P_MIN: f32 = 0.0;
const TOP_P_MAX: f32 = 1.0;

/// Message role enum for strict validation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

impl From<String> for Role {
    fn from(s: String) -> Self {
        match s.to_lowercase().as_str() {
            "system" => Role::System,
            "assistant" => Role::Assistant,
            _ => Role::User, // Default to user for unknown roles
        }
    }
}

impl ToString for Role {
    fn to_string(&self) -> String {
        match self {
            Role::System => "system".to_string(),
            Role::User => "user".to_string(),
            Role::Assistant => "assistant".to_string(),
        }
    }
}

/// Chat message with role and content
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    /// Validate message
    pub fn validate(&self) -> Result<()> {
        if self.content.is_empty() {
            return Err(Error::BadRequest("Message content cannot be empty".into()));
        }
        if self.content.len() > 100_000 {
            return Err(Error::BadRequest("Message content too long (max 100k chars)".into()));
        }
        Ok(())
    }
}

/// Request for chat completion with validation
#[derive(Debug, Clone, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: Option<String>,
    pub messages: Vec<Message>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<usize>,
    pub stream: Option<bool>,
    pub top_p: Option<f32>,
    pub top_k: Option<i32>,
    pub repeat_penalty: Option<f32>,
}

impl ChatCompletionRequest {
    /// Validate the request
    pub fn validate(&self) -> Result<()> {
        // Validate messages
        if self.messages.is_empty() {
            return Err(Error::BadRequest("Messages array cannot be empty".into()));
        }
        
        for msg in &self.messages {
            msg.validate()?;
        }
        
        // Validate temperature
        if let Some(temp) = self.temperature {
            if temp < TEMPERATURE_MIN || temp > TEMPERATURE_MAX {
                return Err(Error::BadRequest(
                    format!("Temperature must be between {} and {}", TEMPERATURE_MIN, TEMPERATURE_MAX)
                ));
            }
        }
        
        // Validate max_tokens
        if let Some(max_tokens) = self.max_tokens {
            if max_tokens < MIN_TOKENS || max_tokens > MAX_TOKENS_LIMIT {
                return Err(Error::BadRequest(
                    format!("max_tokens must be between {} and {}", MIN_TOKENS, MAX_TOKENS_LIMIT)
                ));
            }
        }
        
        // Validate top_p
        if let Some(top_p) = self.top_p {
            if top_p < TOP_P_MIN || top_p > TOP_P_MAX {
                return Err(Error::BadRequest(
                    format!("top_p must be between {} and {}", TOP_P_MIN, TOP_P_MAX)
                ));
            }
        }
        
        // Validate top_k
        if let Some(top_k) = self.top_k {
            if top_k < 1 {
                return Err(Error::BadRequest("top_k must be at least 1".into()));
            }
        }
        
        // Validate repeat_penalty
        if let Some(penalty) = self.repeat_penalty {
            if penalty < 0.1 || penalty > 2.0 {
                return Err(Error::BadRequest("repeat_penalty must be between 0.1 and 2.0".into()));
            }
        }
        
        Ok(())
    }
}

/// Response for chat completion
#[derive(Debug, Clone, Serialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<Choice>,
    pub usage: Usage,
}

/// Choice in completion response
#[derive(Debug, Clone, Serialize)]
pub struct Choice {
    pub index: usize,
    pub message: Message,
    pub finish_reason: Option<FinishReason>,
}

/// Finish reason enum
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ContentFilter,
    Cancelled,
    Error,
}

/// Token usage statistics
#[derive(Debug, Clone, Serialize, Default)]
pub struct Usage {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,
}

/// Streaming frame for SSE
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamFrame {
    /// Start of stream with role
    Start {
        id: String,
        model: String,
        role: Role,
    },
    /// Delta content chunk
    Delta {
        content: String,
    },
    /// End of stream with usage stats
    Done {
        finish_reason: FinishReason,
        usage: Usage,
    },
    /// Error during streaming
    Error {
        message: String,
    },
}

/// Streaming chunk for OpenAI compatibility
#[derive(Debug, Clone, Serialize)]
pub struct ChatCompletionChunk {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<StreamChoice>,
}

/// Streaming choice
#[derive(Debug, Clone, Serialize)]
pub struct StreamChoice {
    pub index: usize,
    pub delta: DeltaContent,
    pub finish_reason: Option<FinishReason>,
}

/// Delta content for streaming
#[derive(Debug, Clone, Serialize)]
pub struct DeltaContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

/// Health check response
#[derive(Debug, Clone, Serialize)]
pub struct HealthResponse {
    pub status: HealthStatus,
    pub model_loaded: bool,
    pub version: String,
    pub uptime_seconds: u64,
}

/// Health status enum
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

/// Generation parameters for runtime
#[derive(Debug, Clone)]
pub struct GenerationParams {
    pub request_id: String,
    pub temperature: f32,
    pub max_tokens: usize,
    pub top_p: f32,
    pub top_k: i32,
    pub repeat_penalty: f32,
    pub stop_sequences: Vec<String>,
}

impl GenerationParams {
    /// Create from request with defaults
    pub fn from_request(req: &ChatCompletionRequest, defaults: GenerationParams) -> Self {
        Self {
            request_id: uuid::Uuid::new_v4().to_string(),
            temperature: req.temperature.unwrap_or(defaults.temperature),
            max_tokens: req.max_tokens.unwrap_or(defaults.max_tokens),
            top_p: req.top_p.unwrap_or(defaults.top_p),
            top_k: req.top_k.unwrap_or(defaults.top_k),
            repeat_penalty: req.repeat_penalty.unwrap_or(defaults.repeat_penalty),
            stop_sequences: defaults.stop_sequences,
        }
    }
}

impl Default for GenerationParams {
    fn default() -> Self {
        Self {
            request_id: uuid::Uuid::new_v4().to_string(),
            temperature: 0.6,
            max_tokens: 256,
            top_p: 0.9,
            top_k: 40,
            repeat_penalty: 1.15,
            stop_sequences: vec![
                "<|eot_id|>".to_string(),
                "<|end_of_text|>".to_string(),
                "<|start_header_id|>".to_string(),
            ],
        }
    }
}