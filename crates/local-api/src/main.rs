use anyhow::Result;
use axum::{
    extract::State,
    response::{sse::Event, IntoResponse, Sse},
    routing::{get, post},
    Json, Router,
};
use futures::stream::Stream;
use infer_runtime::{InferenceConfig, InferenceRuntime};
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, net::SocketAddr, sync::Arc};
use tokio::sync::RwLock;
use tokio_stream::{self as stream, StreamExt};
use tower_http::trace::TraceLayer;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone)]
struct AppState {
    runtime: Arc<RwLock<InferenceRuntime>>,
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

    // Load configuration
    let model_path = std::env::var("MODEL_PATH")
        .unwrap_or_else(|_| "./llama.cpp/models/tinyllama.gguf".to_string());
    
    let config = InferenceConfig {
        model_path,
        context_size: 2048,
        threads: 4,
        temperature: 0.7,
        max_tokens: 512,
        server_port: 8080,  // llama-server port
    };

    // Start the llama-server subprocess
    let mut runtime = InferenceRuntime::new(config);
    runtime.start_server().await?;

    let state = AppState {
        runtime: Arc::new(RwLock::new(runtime)),
    };

    // Build router with privacy guardrails
    let app = Router::new()
        .route("/healthz", get(health_check))
        .route("/v1/chat/completions", post(chat_completions))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    // CRITICAL: Bind to localhost only for privacy
    let addr = SocketAddr::from(([127, 0, 0, 1], 8081));
    info!("Listening on http://{} (localhost only)", addr);
    
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn health_check() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "chatsafe-local-api"
    }))
}

async fn chat_completions(
    State(state): State<AppState>,
    Json(request): Json<ChatCompletionRequest>,
) -> impl IntoResponse {
    let stream = request.stream.unwrap_or(true);
    
    // Format messages into a prompt with TinyLlama template
    let prompt = format_messages(&request.messages);
    
    let temperature = request.temperature.unwrap_or(0.7);
    let max_tokens = request.max_tokens.unwrap_or(512);
    
    if stream {
        // For now, return a simple non-streaming response wrapped as SSE
        // (Full streaming would require using llama-server's SSE endpoint)
        let runtime = state.runtime.read().await;
        match runtime.complete(prompt.clone(), temperature, max_tokens).await {
            Ok(response) => {
                let content = clean_response(&response.content);
                let stream = create_simple_sse_stream(content);
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
                    model: "tinyllama".to_string(),
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

fn create_simple_sse_stream(content: String) -> impl Stream<Item = Result<Event, Infallible>> {
    let chunk = ChatCompletionChunk {
        id: format!("chatcmpl-{}", uuid::Uuid::new_v4()),
        object: "chat.completion.chunk".to_string(),
        created: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        model: "tinyllama".to_string(),
        choices: vec![ChunkChoice {
            index: 0,
            delta: Delta {
                content: Some(content),
                role: None,
            },
            finish_reason: None,
        }],
    };
    
    stream::once(Ok(Event::default().data(serde_json::to_string(&chunk).unwrap_or_default())))
        .chain(stream::once(Ok(Event::default().data("[DONE]"))))
}

fn create_error_sse_stream(error: String) -> impl Stream<Item = Result<Event, Infallible>> {
    stream::once(Ok(Event::default().data(serde_json::json!({
        "error": error
    }).to_string())))
}

fn format_messages(messages: &[Message]) -> String {
    // Use TinyLlama's chat template format
    let mut formatted = String::new();
    
    // Add a default system prompt if none provided
    let has_system = messages.iter().any(|m| m.role == "system");
    if !has_system {
        formatted.push_str("<|system|>\nA private AI assistant running entirely on this device. You provide helpful, accurate, and concise responses. Be direct and helpful.</s>\n");
    }
    
    for msg in messages {
        match msg.role.as_str() {
            "system" => {
                formatted.push_str(&format!("<|system|>\n{}</s>\n", msg.content));
            }
            "user" => {
                formatted.push_str(&format!("<|user|>\n{}</s>\n", msg.content));
            }
            "assistant" => {
                formatted.push_str(&format!("<|assistant|>\n{}</s>\n", msg.content));
            }
            _ => {}
        }
    }
    
    // Add the assistant prompt to trigger response
    formatted.push_str("<|assistant|>\n");
    formatted
}

fn clean_response(content: &str) -> String {
    // Remove any template artifacts from the response
    content
        .lines()
        .filter(|line| {
            !line.contains("<|system|>") && 
            !line.contains("<|user|>") && 
            !line.contains("<|assistant|>") &&
            !line.contains("</s>") &&
            !line.trim().is_empty()
        })
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

use uuid;