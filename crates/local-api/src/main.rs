use anyhow::Result;
use axum::{
    extract::State,
    response::{sse::Event, IntoResponse, Sse},
    routing::{get, post},
    Json, Router,
};
use futures::stream::Stream;
use futures::StreamExt;
use std::pin::Pin;
use infer_runtime::{InferenceConfig, InferenceRuntime};
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, net::SocketAddr, sync::Arc, path::PathBuf};
use tokio::sync::RwLock;
use tokio_stream as stream;
use tower_http::trace::TraceLayer;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

// Model configuration constants
const MODEL_ID: &str = "llama-3.2-3b-instruct-q4_k_m";
const MODEL_FILENAME: &str = "llama-3.2-3b-instruct-q4_k_m.gguf";
const CONTEXT_WINDOW: usize = 8192;
const SYSTEM_PROMPT: &str = "You are a concise, helpful assistant. Answer directly and briefly unless asked for detail.";

// Default parameters
const DEFAULT_TEMPERATURE: f32 = 0.6;
const DEFAULT_TOP_P: f32 = 0.9;
const DEFAULT_TOP_K: i32 = 40;
const DEFAULT_REPEAT_PENALTY: f32 = 1.15;
const DEFAULT_MAX_TOKENS: usize = 256;

#[derive(Clone)]
struct AppState {
    runtime: Arc<RwLock<InferenceRuntime>>,
    model_id: String,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionRequest {
    model: Option<String>,
    messages: Vec<Message>,
    temperature: Option<f32>,
    max_tokens: Option<usize>,
    stream: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct ChatCompletionResponse {
    id: String,
    object: String,
    created: u64,
    model: String,
    choices: Vec<Choice>,
    usage: Usage,
}

#[derive(Serialize)]
struct Choice {
    index: usize,
    message: Message,
    finish_reason: String,
}

#[derive(Serialize)]
struct Usage {
    prompt_tokens: usize,
    completion_tokens: usize,
    total_tokens: usize,
}

#[derive(Serialize)]
struct ChatCompletionChunk {
    id: String,
    object: String,
    created: u64,
    model: String,
    choices: Vec<ChunkChoice>,
}

#[derive(Serialize)]
struct ChunkChoice {
    index: usize,
    delta: Delta,
    finish_reason: Option<String>,
}

#[derive(Serialize)]
struct Delta {
    content: Option<String>,
    role: Option<String>,
}

fn get_model_path() -> PathBuf {
    // Check for environment variable override first
    if let Ok(path) = std::env::var("MODEL_PATH") {
        return PathBuf::from(path);
    }
    
    // Default to app data directory
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home)
        .join(".local/share/chatsafe/models")
        .join(MODEL_FILENAME)
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting ChatSafe local API server");

    let model_path = get_model_path();
    
    // Check if model file exists
    if !model_path.exists() {
        error!("Model file not found at: {}", model_path.display());
        eprintln!(
            "\n⚠️  Model file not found!\n\n\
            Please download the Llama-3.2-3B-Instruct Q4_K_M model and place it at:\n\
            {}\n\n\
            You can download it from HuggingFace or use:\n\
            wget https://huggingface.co/bartowski/Llama-3.2-3B-Instruct-GGUF/resolve/main/Llama-3.2-3B-Instruct-Q4_K_M.gguf\n",
            model_path.display()
        );
        std::process::exit(1);
    }
    
    info!("Using model: {}", model_path.display());
    
    let config = InferenceConfig {
        model_path: model_path.to_string_lossy().to_string(),
        context_size: CONTEXT_WINDOW,
        threads: 4,
        temperature: DEFAULT_TEMPERATURE,
        max_tokens: DEFAULT_MAX_TOKENS,
        server_port: 8080,  // llama-server port
    };

    // Start the llama-server subprocess
    let mut runtime = InferenceRuntime::new(config);
    runtime.start_server().await?;

    let state = AppState {
        runtime: Arc::new(RwLock::new(runtime)),
        model_id: MODEL_ID.to_string(),
    };

    // Build router with privacy guardrails
    let app = Router::new()
        .route("/healthz", get(health_check))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/models", get(list_models))
        .route("/version", get(version_info))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    // CRITICAL: Bind to localhost only for privacy
    let addr = SocketAddr::from(([127, 0, 0, 1], 8081));
    info!("Listening on http://{} (localhost only)", addr);
    
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn health_check(State(state): State<AppState>) -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "chatsafe-local-api",
        "model": state.model_id
    }))
}

async fn version_info(State(state): State<AppState>) -> impl IntoResponse {
    Json(serde_json::json!({
        "model_id": state.model_id,
        "context_window": CONTEXT_WINDOW,
        "defaults": {
            "temperature": DEFAULT_TEMPERATURE,
            "top_p": DEFAULT_TOP_P,
            "top_k": DEFAULT_TOP_K,
            "repeat_penalty": DEFAULT_REPEAT_PENALTY,
            "max_tokens": DEFAULT_MAX_TOKENS
        }
    }))
}

async fn list_models(State(state): State<AppState>) -> impl IntoResponse {
    Json(serde_json::json!({
        "object": "list",
        "data": [{
            "id": state.model_id,
            "object": "model",
            "created": 1700000000,  // Placeholder timestamp
            "owned_by": "local",
            "permission": [],
            "root": state.model_id,
            "parent": null,
        }]
    }))
}

async fn chat_completions(
    State(state): State<AppState>,
    Json(request): Json<ChatCompletionRequest>,
) -> impl IntoResponse {
    let stream = request.stream.unwrap_or(true);
    
    // Format messages into a prompt with Llama-3 template
    let prompt = format_messages(&request.messages);
    
    let temperature = request.temperature.unwrap_or(DEFAULT_TEMPERATURE);
    let max_tokens = request.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);
    
    if stream {
        // Use llama-server's streaming endpoint for real token-by-token streaming
        let runtime = state.runtime.read().await;
        match runtime.complete_stream(prompt.clone(), temperature, max_tokens).await {
            Ok(chunk_stream) => {
                let stream = create_proper_sse_stream(chunk_stream, state.model_id.clone());
                Sse::new(stream).into_response()
            }
            Err(e) => {
                error!("Completion error: {}", e);
                let error_stream = create_error_sse_stream(e.to_string());
                Sse::new(error_stream).into_response()
            }
        }
    } else {
        // Non-streaming response
        let runtime = state.runtime.read().await;
        match runtime.complete(prompt.clone(), temperature, max_tokens).await {
            Ok(response) => {
                let content = clean_response(&response.content);
                let response = ChatCompletionResponse {
                    id: format!("chatcmpl-{}", uuid::Uuid::new_v4()),
                    object: "chat.completion".to_string(),
                    created: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                    model: state.model_id.clone(),
                    choices: vec![Choice {
                        index: 0,
                        message: Message {
                            role: "assistant".to_string(),
                            content: content.clone(),
                        },
                        finish_reason: "stop".to_string(),
                    }],
                    usage: Usage {
                        prompt_tokens: prompt.split_whitespace().count(),
                        completion_tokens: content.split_whitespace().count(),
                        total_tokens: prompt.split_whitespace().count() + content.split_whitespace().count(),
                    },
                };
                Json(response).into_response()
            }
            Err(e) => {
                error!("Completion error: {}", e);
                Json(serde_json::json!({
                    "error": {
                        "message": e.to_string(),
                        "type": "inference_error"
                    }
                })).into_response()
            }
        }
    }
}

fn create_proper_sse_stream(
    mut chunk_stream: Pin<Box<dyn Stream<Item = Result<infer_runtime::StreamChunk, anyhow::Error>> + Send>>,
    model_id: String,
) -> impl Stream<Item = Result<Event, Infallible>> {
    let id = format!("chatcmpl-{}", uuid::Uuid::new_v4());
    let created = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    
    async_stream::stream! {
        // First chunk: send role
        let first_chunk = ChatCompletionChunk {
            id: id.clone(),
            object: "chat.completion.chunk".to_string(),
            created,
            model: model_id.clone(),
            choices: vec![ChunkChoice {
                index: 0,
                delta: Delta {
                    content: None,
                    role: Some("assistant".to_string()),
                },
                finish_reason: None,
            }],
        };
        yield Ok(Event::default().data(serde_json::to_string(&first_chunk).unwrap_or_default()));
        
        // Stream content chunks
        let mut buffer = String::new();
        while let Some(result) = chunk_stream.next().await {
            match result {
                Ok(chunk) => {
                    let content = clean_response(&chunk.content);
                    if !content.is_empty() {
                        buffer.push_str(&content);
                        
                        // Send chunk with content delta
                        let chunk_msg = ChatCompletionChunk {
                            id: id.clone(),
                            object: "chat.completion.chunk".to_string(),
                            created,
                            model: model_id.clone(),
                            choices: vec![ChunkChoice {
                                index: 0,
                                delta: Delta {
                                    content: Some(content),
                                    role: None,
                                },
                                finish_reason: None,
                            }],
                        };
                        yield Ok(Event::default().data(serde_json::to_string(&chunk_msg).unwrap_or_default()));
                    }
                    
                    // Check if this is the final chunk
                    if chunk.stop.unwrap_or(false) {
                        // Send finish reason chunk
                        let finish_chunk = ChatCompletionChunk {
                            id: id.clone(),
                            object: "chat.completion.chunk".to_string(),
                            created,
                            model: model_id.clone(),
                            choices: vec![ChunkChoice {
                                index: 0,
                                delta: Delta {
                                    content: None,
                                    role: None,
                                },
                                finish_reason: Some("stop".to_string()),
                            }],
                        };
                        yield Ok(Event::default().data(serde_json::to_string(&finish_chunk).unwrap_or_default()));
                        break;
                    }
                }
                Err(e) => {
                    error!("Stream error: {}", e);
                    break;
                }
            }
        }
        
        // Send [DONE] marker
        yield Ok(Event::default().data("[DONE]"));
    }
}

fn create_error_sse_stream(error: String) -> impl Stream<Item = Result<Event, Infallible>> {
    stream::once(Ok(Event::default().data(serde_json::json!({
        "error": error
    }).to_string())))
}

fn format_messages(messages: &[Message]) -> String {
    // Use Llama-3 Instruct chat template format
    let mut formatted = String::new();
    
    // Start with begin of text marker
    formatted.push_str("<|begin_of_text|>");
    
    // Add system prompt (always include one)
    let has_system = messages.iter().any(|m| m.role == "system");
    if !has_system {
        formatted.push_str("<|start_header_id|>system<|end_header_id|>\n\n");
        formatted.push_str(SYSTEM_PROMPT);
        formatted.push_str("<|eot_id|>");
    }
    
    for msg in messages {
        match msg.role.as_str() {
            "system" => {
                formatted.push_str("<|start_header_id|>system<|end_header_id|>\n\n");
                formatted.push_str(&msg.content);
                formatted.push_str("<|eot_id|>");
            }
            "user" => {
                formatted.push_str("<|start_header_id|>user<|end_header_id|>\n\n");
                formatted.push_str(&msg.content);
                formatted.push_str("<|eot_id|>");
            }
            "assistant" => {
                formatted.push_str("<|start_header_id|>assistant<|end_header_id|>\n\n");
                formatted.push_str(&msg.content);
                formatted.push_str("<|eot_id|>");
            }
            _ => {}
        }
    }
    
    // Add assistant header to trigger response
    formatted.push_str("<|start_header_id|>assistant<|end_header_id|>\n\n");
    formatted
}

fn clean_response(content: &str) -> String {
    // Remove any Llama-3 template artifacts from the response
    let mut cleaned = content.to_string();
    
    // Remove Llama-3 specific template markers
    let template_markers = [
        "<|eot_id|>",
        "<|end_of_text|>",
        "<|start_header_id|>",
        "<|end_header_id|>",
        "<|begin_of_text|>",
    ];
    
    for marker in &template_markers {
        cleaned = cleaned.replace(marker, "");
    }
    
    // Remove role headers ONLY when they appear with template markers
    // Pattern: <|start_header_id|>role<|end_header_id|>
    cleaned = cleaned
        .replace("<|start_header_id|>assistant<|end_header_id|>", "")
        .replace("<|start_header_id|>user<|end_header_id|>", "")
        .replace("<|start_header_id|>system<|end_header_id|>", "");
    
    // Remove common role pollution patterns that shouldn't appear
    let role_pollution_patterns = [
        "AI: ",
        "AI:",
        "Assistant: ",
        "Assistant:",
        "You: ",
        "You:",
        "User: ",
        "User:",
        "Human: ",
        "Human:",
    ];
    
    for pattern in &role_pollution_patterns {
        // Only remove if at the beginning of a line
        cleaned = cleaned
            .lines()
            .map(|line| {
                if line.starts_with(pattern) {
                    line[pattern.len()..].to_string()
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    
    // Clean up excessive whitespace but preserve paragraph structure
    cleaned
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

use uuid;