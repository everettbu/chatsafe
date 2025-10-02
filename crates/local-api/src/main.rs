use anyhow::Result;

mod rate_limiter;
mod streaming;
#[cfg(test)]
mod tests;
use axum::{
    extract::{State, ConnectInfo},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
    http::{HeaderValue, StatusCode},
};
use chatsafe_common::{
    ChatCompletionRequest, ChatCompletionResponse,
    Message, Choice, Usage, Role, FinishReason,
    HealthResponse, HealthStatus, GenerationParams, StreamFrame,
    Error as CommonError, ErrorResponse, ObservableMetrics, RequestId,
    ObservableMetricsSnapshot,
};
use chatsafe_config::{ConfigLoader, ModelRegistry};
use chatsafe_runtime::{RuntimeHandle, ModelRuntime, ModelHandle};
use futures::StreamExt;
use serde_json::json;
use std::{net::SocketAddr, sync::Arc};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tower_http::trace::TraceLayer;
use rate_limiter::{RateLimiter, RateLimiterConfig};
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

// Constants
const API_VERSION: &str = "0.1.0";
const HEALTH_CHECK_TIMEOUT_SECS: u64 = 2;
const REQUEST_ID_HEADER: &str = "x-request-id";
const DEFAULT_MODEL_NAME: &str = "unknown";
const CHAT_COMPLETION_OBJECT: &str = "chat.completion";

#[derive(Clone)]
struct AppState {
    runtime: RuntimeHandle,
    registry: Arc<ModelRegistry>,
    model_handle: Arc<RwLock<Option<ModelHandle>>>,
    start_time: SystemTime,
    metrics: Arc<ObservableMetrics>,
    rate_limiter: RateLimiter,
}

// Helper function to create error response with request ID
fn create_error_response(
    error: &CommonError,
    request_id: &RequestId,
    status: StatusCode,
) -> Response {
    let mut error_response = ErrorResponse::from(error);
    error_response.request_id = Some(request_id.to_string());
    
    (
        status,
        [
            (
                axum::http::header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (
                axum::http::HeaderName::from_static(REQUEST_ID_HEADER),
                HeaderValue::from_str(&request_id.to_string())
                    .unwrap_or_else(|_| HeaderValue::from_static(DEFAULT_MODEL_NAME)),
            ),
        ],
        Json(error_response),
    )
        .into_response()
}

// Helper to add request ID header to response
fn add_request_id_header(response: &mut Response, request_id: &RequestId) {
    response.headers_mut().insert(
        axum::http::HeaderName::from_static(REQUEST_ID_HEADER),
        HeaderValue::from_str(&request_id.to_string())
            .unwrap_or_else(|_| HeaderValue::from_static(DEFAULT_MODEL_NAME)),
    );
}

async fn health_check(State(state): State<AppState>) -> Json<HealthResponse> {
    // Apply timeout to health check
    let health_future = state.runtime.health();
    let timeout_duration = Duration::from_secs(HEALTH_CHECK_TIMEOUT_SECS);
    
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
        version: API_VERSION.to_string(),
        uptime_seconds: uptime,
    })
}

// Handle streaming response
async fn handle_streaming(
    state: &AppState,
    handle: &ModelHandle,
    messages: Vec<Message>,
    params: GenerationParams,
    request_id: &RequestId,
    tracked_request_id: &RequestId,
    ip: std::net::IpAddr,
) -> Result<Response, Response> {
    let model_id = handle.model_id.to_string();
    
    let stream = state.runtime.generate(handle, messages, params)
        .await
        .map_err(|e| {
            let response = create_error_response(&e, &request_id, StatusCode::INTERNAL_SERVER_ERROR);
            
            // Complete request tracking on error
            let metrics = Arc::clone(&state.metrics);
            let req_id = request_id.clone();
            let tracked_id = tracked_request_id.clone();
            tokio::spawn(async move {
                metrics.record_error(Some(&req_id), &e).await;
                metrics.complete_request(&tracked_id).await;
            });
            
            response
        })?;
    
    // Request completion is handled by streaming module's CleanupGuard
    let mut response = streaming::streaming_response_with_observability(
        stream,
        model_id,
        Arc::clone(&state.metrics),
        state.rate_limiter.clone(),
        ip,
        tracked_request_id.clone(),
    ).into_response();
    
    add_request_id_header(&mut response, request_id);
    Ok(response)
}

// Handle non-streaming response
async fn handle_non_streaming(
    state: &AppState,
    handle: &ModelHandle,
    messages: Vec<Message>,
    params: GenerationParams,
    request_id: &RequestId,
    tracked_request_id: &RequestId,
    ip: std::net::IpAddr,
) -> Result<Response, Response> {
    let model_id = handle.model_id.to_string();
    
    let mut stream = state.runtime.generate(handle, messages, params.clone())
        .await
        .map_err(|e| {
            let response = create_error_response(&e, &request_id, StatusCode::INTERNAL_SERVER_ERROR);
            
            // Complete request tracking on error
            let metrics = Arc::clone(&state.metrics);
            let req_id = request_id.clone();
            let tracked_id = tracked_request_id.clone();
            tokio::spawn(async move {
                metrics.record_error(Some(&req_id), &e).await;
                metrics.complete_request(&tracked_id).await;
            });
            
            response
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
                // Complete request tracking on error
                state.rate_limiter.release_request(ip).await;
                
                let err = CommonError::RuntimeError(message);
                state.metrics.record_error(Some(request_id), &err).await;
                state.metrics.complete_request(&tracked_request_id).await;
                
                return Err(create_error_response(&err, request_id, StatusCode::INTERNAL_SERVER_ERROR));
            }
            _ => {}
        }
    }
    
    // Create response
    let response = ChatCompletionResponse {
        id: params.request_id,
        object: CHAT_COMPLETION_OBJECT.to_string(),
        created: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
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
    
    // Release rate limit for non-streaming requests
    state.rate_limiter.release_request(ip).await;
    
    // Complete request tracking
    state.metrics.complete_request(&tracked_request_id).await;
    
    // Create response with headers
    let mut http_response = Json(response).into_response();
    add_request_id_header(&mut http_response, request_id);
    
    Ok(http_response)
}

async fn chat_completion(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(request): Json<ChatCompletionRequest>,
) -> Result<Response, Response> {
    let ip = addr.ip();
    
    // Generate request ID for tracing
    let request_id = RequestId::new();
    
    // Start tracking this request early for all paths
    let is_streaming = request.stream.unwrap_or(true);
    let model_name = request.model.clone().unwrap_or_else(|| String::from(DEFAULT_MODEL_NAME));
    let tracked_request_id = state.metrics.start_request(
        request_id.clone(),
        model_name.clone(),
        is_streaming
    ).await;
    
    // Check rate limit
    if let Err(e) = state.rate_limiter.check_rate_limit(ip).await {
        state.metrics.record_error(Some(&request_id), &e).await;
        state.metrics.record_rate_limit(ip.to_string()).await;
        state.metrics.complete_request(&tracked_request_id).await;
        
        return Err(create_error_response(&e, &request_id, StatusCode::TOO_MANY_REQUESTS));
    }
    
    // Validate request
    if let Err(e) = request.validate() {
        state.metrics.record_error(Some(&request_id), &e).await;
        state.metrics.complete_request(&tracked_request_id).await;
        
        return Err(create_error_response(&e, &request_id, StatusCode::BAD_REQUEST));
    }
    
    // Get model handle
    let handle = state.model_handle.read().await.clone()
        .ok_or_else(|| {
            let err = CommonError::RuntimeNotReady;
            let response = create_error_response(&err, &request_id, StatusCode::SERVICE_UNAVAILABLE);
            
            // Record error and complete request tracking
            let metrics = Arc::clone(&state.metrics);
            let req_id = request_id.clone();
            let tracked_id = tracked_request_id.clone();
            tokio::spawn(async move {
                metrics.record_error(Some(&req_id), &err).await;
                metrics.complete_request(&tracked_id).await;
            });
            
            response
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
        let response = create_error_response(&e, &request_id, StatusCode::INTERNAL_SERVER_ERROR);
        
        // Record error and complete request
        let metrics = Arc::clone(&state.metrics);
        let req_id = request_id.clone();
        let tracked_id = tracked_request_id.clone();
        tokio::spawn(async move {
            metrics.record_error(Some(&req_id), &e).await;
            metrics.complete_request(&tracked_id).await;
        });
        
        response
    })?;
    
    // Add request ID to params for tracing
    params.request_id = request_id.to_string();
    
    // Convert messages
    let messages: Vec<Message> = request.messages;
    
    if is_streaming {
        handle_streaming(&state, &handle, messages, params, &request_id, &tracked_request_id, ip).await
    } else {
        handle_non_streaming(&state, &handle, messages, params, &request_id, &tracked_request_id, ip).await
    }
}

async fn version() -> Json<serde_json::Value> {
    Json(json!({
        "version": API_VERSION,
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