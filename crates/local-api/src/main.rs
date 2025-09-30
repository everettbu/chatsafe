use anyhow::Result;
use axum::{
    extract::State,
    response::{sse::Event, IntoResponse, Sse},
    routing::{get, post},
    Json, Router,
};
use chatsafe_common::{
    ChatCompletionRequest, ChatCompletionResponse, ChatCompletionChunk,
    Message, Choice, StreamChoice, DeltaContent, Usage, Role, FinishReason,
    HealthResponse, HealthStatus, GenerationParams, StreamFrame,
    Error as CommonError, ErrorResponse,
};
use chatsafe_config::{ConfigLoader, ModelRegistry, AppConfig};
use chatsafe_runtime::{RuntimeHandle, ModelRuntime, ModelHandle};
use futures::stream::Stream;
use futures::StreamExt;
use serde_json::json;
use std::{convert::Infallible, net::SocketAddr, sync::Arc, path::PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tokio_stream as stream;
use tower_http::trace::TraceLayer;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone)]
struct AppState {
    runtime: RuntimeHandle,
    registry: Arc<ModelRegistry>,
    model_handle: Arc<RwLock<Option<ModelHandle>>>,
    start_time: SystemTime,
}

async fn health_check(State(state): State<AppState>) -> Json<HealthResponse> {
    let health = state.runtime.health().await.unwrap_or_else(|_| {
        chatsafe_runtime::RuntimeHealth {
            is_healthy: false,
            model_loaded: None,
            active_requests: 0,
            uptime_seconds: 0,
        }
    });
    
    let uptime = state.start_time.elapsed()
        .unwrap_or_default()
        .as_secs();
    
    Json(HealthResponse {
        status: if health.is_healthy { 
            HealthStatus::Healthy 
        } else { 
            HealthStatus::Unhealthy 
        },
        model_loaded: health.model_loaded.is_some(),
        version: "0.1.0".to_string(),
        uptime_seconds: uptime,
    })
}

async fn chat_completion(
    State(state): State<AppState>,
    Json(request): Json<ChatCompletionRequest>,
) -> Result<impl IntoResponse, (axum::http::StatusCode, Json<ErrorResponse>)> {
    // Validate request
    if let Err(e) = request.validate() {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            Json(ErrorResponse::from(&e)),
        ));
    }
    
    // Get model handle
    let handle = state.model_handle.read().await.clone()
        .ok_or_else(|| {
            let err = CommonError::RuntimeNotReady;
            (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse::from(&err)),
            )
        })?;
    
    // Get model config and create params
    let model_id = handle.model_id.clone();
    let params = state.registry.apply_overrides(
        &model_id,
        request.temperature,
        request.max_tokens,
        request.top_p,
        request.top_k,
        request.repeat_penalty,
    ).map_err(|e| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::from(&e)),
        )
    })?;
    
    // Convert messages
    let messages: Vec<Message> = request.messages;
    
    if request.stream.unwrap_or(true) {
        // Streaming response
        let stream = state.runtime.generate(&handle, messages, params)
            .await
            .map_err(|e| {
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse::from(&e)),
                )
            })?;
        
        Ok(streaming_response(stream, model_id).into_response())
    } else {
        // Non-streaming response
        let mut stream = state.runtime.generate(&handle, messages, params.clone())
            .await
            .map_err(|e| {
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse::from(&e)),
                )
            })?;
        
        // Collect all frames
        let mut content = String::new();
        let mut usage = Usage::default();
        let mut finish_reason = FinishReason::Stop;
        
        while let Some(frame) = stream.next().await {
            match frame {
                Ok(StreamFrame::Delta { content: delta }) => {
                    content.push_str(&delta);
                }
                Ok(StreamFrame::Done { finish_reason: reason, usage: u }) => {
                    finish_reason = reason;
                    usage = u;
                }
                Ok(StreamFrame::Error { message }) => {
                    return Err((
                        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse::from(&CommonError::RuntimeError(message))),
                    ));
                }
                _ => {}
            }
        }
        
        // Create response
        let response = ChatCompletionResponse {
            id: params.request_id,
            object: "chat.completion".to_string(),
            created: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_else(|_| Duration::from_secs(0))
                .as_secs() as i64,
            model: model_id,
            choices: vec![Choice {
                index: 0,
                message: Message {
                    role: Role::Assistant,
                    content,
                },
                finish_reason: Some(finish_reason),
            }],
            usage,
        };
        
        Ok(Json(response).into_response())
    }
}

fn streaming_response(
    mut stream: std::pin::Pin<Box<dyn Stream<Item = Result<StreamFrame, CommonError>> + Send>>,
    model_id: String,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let response_stream = async_stream::stream! {
        let request_id = uuid::Uuid::new_v4().to_string();
        let created = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_else(|_| Duration::from_secs(0))
            .as_secs() as i64;
        
        while let Some(frame_result) = stream.next().await {
            match frame_result {
                Ok(StreamFrame::Start { role, .. }) => {
                    // Send initial chunk with role
                    let chunk = ChatCompletionChunk {
                        id: request_id.clone(),
                        object: "chat.completion.chunk".to_string(),
                        created,
                        model: model_id.clone(),
                        choices: vec![StreamChoice {
                            index: 0,
                            delta: DeltaContent {
                                role: Some(role),
                                content: None,
                            },
                            finish_reason: None,
                        }],
                    };
                    
                    let data = match serde_json::to_string(&chunk) {
                        Ok(json) => json,
                        Err(e) => {
                            error!("Failed to serialize start chunk: {}", e);
                            continue;
                        }
                    };
                    yield Ok(Event::default().data(data));
                }
                Ok(StreamFrame::Delta { content }) => {
                    // Send content chunk
                    let chunk = ChatCompletionChunk {
                        id: request_id.clone(),
                        object: "chat.completion.chunk".to_string(),
                        created,
                        model: model_id.clone(),
                        choices: vec![StreamChoice {
                            index: 0,
                            delta: DeltaContent {
                                role: None,
                                content: Some(content),
                            },
                            finish_reason: None,
                        }],
                    };
                    
                    let data = match serde_json::to_string(&chunk) {
                        Ok(json) => json,
                        Err(e) => {
                            error!("Failed to serialize delta chunk: {}", e);
                            continue;
                        }
                    };
                    yield Ok(Event::default().data(data));
                }
                Ok(StreamFrame::Done { finish_reason, .. }) => {
                    // Send final chunk
                    let chunk = ChatCompletionChunk {
                        id: request_id.clone(),
                        object: "chat.completion.chunk".to_string(),
                        created,
                        model: model_id.clone(),
                        choices: vec![StreamChoice {
                            index: 0,
                            delta: DeltaContent {
                                role: None,
                                content: None,
                            },
                            finish_reason: Some(finish_reason),
                        }],
                    };
                    
                    let data = match serde_json::to_string(&chunk) {
                        Ok(json) => json,
                        Err(e) => {
                            error!("Failed to serialize final chunk: {}", e);
                            yield Ok(Event::default().data("[DONE]"));
                            break;
                        }
                    };
                    yield Ok(Event::default().data(data));
                    
                    // Send [DONE] marker
                    yield Ok(Event::default().data("[DONE]"));
                }
                Ok(StreamFrame::Error { message }) => {
                    // Send error as data
                    let error_data = json!({
                        "error": {
                            "message": message,
                            "type": "runtime_error"
                        }
                    });
                    yield Ok(Event::default().data(error_data.to_string()));
                    break;
                }
                Err(e) => {
                    // Send error as data
                    let error_data = json!({
                        "error": {
                            "message": e.to_string(),
                            "type": "stream_error"
                        }
                    });
                    yield Ok(Event::default().data(error_data.to_string()));
                    break;
                }
            }
        }
    };
    
    Sse::new(response_stream)
}

async fn version() -> &'static str {
    "ChatSafe API v0.1.0"
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(tracing_subscriber::EnvFilter::from_default_env()
            .add_directive(tracing::Level::INFO.into()))
        .init();

    info!("Starting ChatSafe local API server");
    
    // Load configuration
    let config = ConfigLoader::load(None)?;
    
    // Load model registry
    let registry = ModelRegistry::load_defaults()?;
    
    // Create runtime
    let runtime = ModelRuntime::create(&config, &registry).await?;
    
    // Load default model
    let default_model = registry.get_default_model()?;
    info!("Loading default model: {}", default_model.id);
    
    let model_handle = runtime.load(&default_model.id).await?;
    
    // Create app state
    let state = AppState {
        runtime,
        registry: Arc::new(registry),
        model_handle: Arc::new(RwLock::new(Some(model_handle))),
        start_time: SystemTime::now(),
    };
    
    // Build router
    let app = Router::new()
        .route("/v1/chat/completions", post(chat_completion))
        .route("/healthz", get(health_check))
        .route("/health", get(health_check))
        .route("/version", get(version))
        .layer(TraceLayer::new_for_http())
        .with_state(state);
    
    // Start server
    let addr = SocketAddr::from(([127, 0, 0, 1], config.server.port));
    info!("Listening on http://{} (localhost only)", addr);
    
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    
    Ok(())
}