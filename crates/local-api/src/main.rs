use anyhow::Result;

mod rate_limiter;
mod streaming;
#[cfg(test)]
mod tests;
use axum::{
    extract::{State, ConnectInfo},
    response::{sse::Event, IntoResponse, Sse},
    routing::{get, post},
    Json, Router,
    middleware,
};
use chatsafe_common::{
    ChatCompletionRequest, ChatCompletionResponse, ChatCompletionChunk,
    Message, Choice, StreamChoice, DeltaContent, Usage, Role, FinishReason,
    HealthResponse, HealthStatus, GenerationParams, StreamFrame,
    Error as CommonError, ErrorResponse, ObservableMetrics, RequestId,
    ObservableMetricsSnapshot,
};
use chatsafe_config::{ConfigLoader, ModelRegistry, AppConfig};
use chatsafe_runtime::{RuntimeHandle, ModelRuntime, ModelHandle};
use futures::stream::Stream;
use futures::StreamExt;
use serde_json::json;
use std::{convert::Infallible, net::{SocketAddr, IpAddr}, sync::Arc, path::PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tokio_stream as stream;
use tower_http::trace::TraceLayer;
use rate_limiter::{RateLimiter, RateLimiterConfig};
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone)]
struct AppState {
    runtime: RuntimeHandle,
    registry: Arc<ModelRegistry>,
    model_handle: Arc<RwLock<Option<ModelHandle>>>,
    start_time: SystemTime,
    metrics: Arc<ObservableMetrics>,
    rate_limiter: RateLimiter,
}

async fn health_check(State(state): State<AppState>) -> Json<HealthResponse> {
    // Apply 2-second timeout to health check
    let health_future = state.runtime.health();
    let timeout_duration = Duration::from_secs(2);
    
    let health = match tokio::time::timeout(timeout_duration, health_future).await {
        Ok(Ok(health)) => health,
        Ok(Err(_)) | Err(_) => {
            // Either runtime error or timeout - treat as unhealthy
            chatsafe_runtime::RuntimeHealth {
                is_healthy: false,
                model_loaded: None,
                active_requests: 0,
                uptime_seconds: 0,
            }
        }
    };
    
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
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(request): Json<ChatCompletionRequest>,
) -> Result<impl IntoResponse, (axum::http::StatusCode, Json<ErrorResponse>)> {
    let start_time = std::time::Instant::now();
    let ip = addr.ip();
    
    // Generate request ID for tracing
    let request_id = RequestId::new();
    
    // Check rate limit
    if let Err(e) = state.rate_limiter.check_rate_limit(ip).await {
        state.metrics.record_error(Some(&request_id), &e).await;
        state.metrics.record_rate_limit(ip.to_string()).await;
        
        let mut response = ErrorResponse::from(&e);
        response.request_id = Some(request_id.to_string());
        
        return Err((
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            Json(response),
        ));
    }
    
    // Validate request
    if let Err(e) = request.validate() {
        state.metrics.record_error(Some(&request_id), &e).await;
        
        let mut response = ErrorResponse::from(&e);
        response.request_id = Some(request_id.to_string());
        
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            Json(response),
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
    let model_id = &handle.model_id;
    let mut params = state.registry.apply_overrides(
        model_id,
        request.temperature,
        request.max_tokens,
        request.top_p,
        request.top_k,
        request.repeat_penalty,
    ).map_err(|e| {
        // Record error in background
        {
            let metrics = state.metrics.clone();
            let req_id = request_id.clone();
            let error_type = CommonError::ConfigError(e.to_string());
            tokio::spawn(async move {
                metrics.record_error(Some(&req_id), &error_type).await;
            });
        }
        
        let mut response = ErrorResponse::from(&e);
        response.request_id = Some(request_id.to_string());
        
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(response),
        )
    })?;
    
    // Add request ID to params for tracing
    params.request_id = request_id.to_string();
    
    // Start tracking this request
    let is_streaming = request.stream.unwrap_or(true);
    let tracked_request_id = state.metrics.start_request(
        request_id.clone(),
        model_id.to_string(),
        is_streaming
    ).await;
    
    // Convert messages
    let messages: Vec<Message> = request.messages;
    
    if is_streaming {
        // Streaming response
        let stream = state.runtime.generate(&handle, messages, params)
            .await
            .map_err(|e| {
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse::from(&e)),
                )
            })?;
        
        // Request completion is handled by streaming module's CleanupGuard
        
        Ok(streaming::streaming_response_with_observability(
            stream,
            model_id.to_string(),
            state.metrics.clone(),
            state.rate_limiter.clone(),
            ip,
            tracked_request_id.clone()
        ).into_response())
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
            model: model_id.to_string(),
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
        
        // Release rate limit for non-streaming requests
        state.rate_limiter.release_request(ip).await;
        
        // Complete request tracking
        state.metrics.complete_request(&tracked_request_id).await;
        
        Ok(Json(response).into_response())
    }
}

// Old streaming_response removed - using streaming::streaming_response_with_observability

// NOTE: The old streaming_response function has been removed.
// All streaming is now handled via streaming::streaming_response_with_observability
// which includes proper request tracking, backpressure, and observability.

async fn version() -> Json<serde_json::Value> {
    Json(json!({
        "version": "0.1.0",
        "api": "ChatSafe Local API",
        "model_api": "OpenAI Compatible"
    }))
}

async fn get_models(State(state): State<AppState>) -> Json<serde_json::Value> {
    let models = state.registry.list_models();
    let model_info: Vec<serde_json::Value> = models.iter().map(|id| {
        if let Ok(model) = state.registry.get_model(id) {
            json!({
                "id": model.id,
                "name": model.name,
                "context_window": model.ctx_window,
                "default": model.default
            })
        } else {
            json!({"id": id})
        }
    }).collect();
    
    Json(json!({
        "models": model_info
    }))
}

async fn get_metrics(State(state): State<AppState>) -> Json<ObservableMetricsSnapshot> {
    Json(state.metrics.snapshot().await)
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
    
    // Create rate limiter
    let rate_limiter = RateLimiter::new(RateLimiterConfig::default());
    
    // Create app state
    let state = AppState {
        runtime,
        registry: Arc::new(registry),
        model_handle: Arc::new(RwLock::new(Some(model_handle))),
        start_time: SystemTime::now(),
        metrics: Arc::new(ObservableMetrics::new()),
        rate_limiter,
    };
    
    // Build router with tracing layer
    let app = Router::new()
        .route("/v1/chat/completions", post(chat_completion))
        .route("/healthz", get(health_check))
        .route("/health", get(health_check))
        .route("/version", get(version))
        .route("/metrics", get(get_metrics))
        .route("/models", get(get_models))
        .layer(TraceLayer::new_for_http())
        .with_state(state);
    
    // Start server
    let addr = SocketAddr::from(([127, 0, 0, 1], config.server.port));
    info!("Listening on http://{} (localhost only)", addr);
    
    let listener = tokio::net::TcpListener::bind(addr).await?;
    
    // Use into_make_service_with_connect_info to get client IP addresses
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>()
    )
    .await?;
    
    Ok(())
}