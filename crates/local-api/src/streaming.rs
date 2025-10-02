use axum::response::sse::{Event, Sse};
use chatsafe_common::{
    ChatCompletionChunk, StreamChoice, DeltaContent, StreamFrame,
    Error as CommonError, ObservableMetrics, RequestId,
};
use futures::stream::Stream;
use futures::StreamExt;
use serde_json::json;
use std::convert::Infallible;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::error;

use crate::rate_limiter::RateLimiter;

// Constants
const BUFFER_SIZE: usize = 32;  // Maximum chunks to buffer for backpressure
const CHUNK_TIMEOUT: Duration = Duration::from_secs(30);  // Timeout per chunk
const CHUNK_OBJECT_TYPE: &str = "chat.completion.chunk";
const DONE_MARKER: &str = "[DONE]";
const ERROR_TYPE_RUNTIME: &str = "runtime_error";
const ERROR_TYPE_STREAM: &str = "stream_error";

/// Create an SSE streaming response with observability and backpressure control
/// 
/// This function sets up a bounded channel to prevent memory growth from slow clients,
/// and ensures proper cleanup of rate limits and request tracking.
pub fn streaming_response_with_observability(
    stream: std::pin::Pin<Box<dyn Stream<Item = Result<StreamFrame, CommonError>> + Send>>,
    model_id: String,
    metrics: Arc<ObservableMetrics>,
    rate_limiter: RateLimiter,
    client_ip: IpAddr,
    request_id: RequestId,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // Use bounded channel for backpressure
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(BUFFER_SIZE);
    
    // Spawn producer task with automatic cleanup
    tokio::spawn(async move {
        produce_stream_events(stream, model_id, metrics, tx, rate_limiter, client_ip, request_id).await;
    });
    
    // Consumer stream that yields from the bounded channel
    let response_stream = async_stream::stream! {
        while let Some(event) = rx.recv().await {
            yield event;
        }
    };
    
    Sse::new(response_stream)
}

/// Produce SSE events from the generation stream
async fn produce_stream_events(
    mut stream: std::pin::Pin<Box<dyn Stream<Item = Result<StreamFrame, CommonError>> + Send>>,
    model_id: String,
    metrics: Arc<ObservableMetrics>,
    tx: tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
    rate_limiter: RateLimiter,
    client_ip: IpAddr,
    request_id: RequestId,
) {
    // Ensure cleanup happens when function exits
    let _cleanup = CleanupGuard::new(rate_limiter.clone(), client_ip, metrics.clone(), request_id.clone());
    
    let request_id_str = Arc::new(request_id.to_string());
    let model_id = Arc::new(model_id);
    let created = get_unix_timestamp();
    
    let mut first_token_recorded = false;
    let stream_start = std::time::Instant::now();
    
    while let Some(frame_result) = tokio::time::timeout(
        CHUNK_TIMEOUT,
        stream.next()
    ).await.ok().flatten() {
        let should_continue = process_stream_frame(
            frame_result,
            &tx,
            &request_id_str,
            &model_id,
            created,
            &metrics,
            &mut first_token_recorded,
            stream_start,
        ).await;
        
        if !should_continue {
            break;
        }
    }
}

/// Process a single stream frame and send appropriate SSE event
/// Returns false if streaming should stop
async fn process_stream_frame(
    frame_result: Result<StreamFrame, CommonError>,
    tx: &tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
    request_id: &Arc<String>,
    model_id: &Arc<String>,
    created: i64,
    metrics: &Arc<ObservableMetrics>,
    first_token_recorded: &mut bool,
    stream_start: std::time::Instant,
) -> bool {
    match frame_result {
        Ok(StreamFrame::Start { role, .. }) => {
            send_start_chunk(tx, request_id, model_id, created, role).await
        }
        Ok(StreamFrame::Delta { content }) => {
            // Record first token latency if needed
            if !*first_token_recorded {
                *first_token_recorded = true;
                let latency_ms = stream_start.elapsed().as_millis() as u64;
                metrics.record_first_token_latency(latency_ms).await;
            }
            
            // Track chunk sent
            metrics.record_chunk().await;
            
            send_delta_chunk(tx, request_id, model_id, created, content).await
        }
        Ok(StreamFrame::Done { finish_reason, .. }) => {
            send_done_chunk(tx, request_id, model_id, created, finish_reason).await;
            false // Stop streaming
        }
        Ok(StreamFrame::Error { message }) => {
            send_error_event(tx, message, ERROR_TYPE_RUNTIME).await;
            false // Stop streaming
        }
        Err(e) => {
            send_error_event(tx, e.to_string(), ERROR_TYPE_STREAM).await;
            false // Stop streaming
        }
    }
}

/// Send the initial chunk with role information
async fn send_start_chunk(
    tx: &tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
    request_id: &Arc<String>,
    model_id: &Arc<String>,
    created: i64,
    role: chatsafe_common::Role,
) -> bool {
    let chunk = create_chunk(
        request_id,
        model_id,
        created,
        Some(role),
        None,
        None,
    );
    
    send_chunk_event(tx, chunk).await
}

/// Send a delta chunk with content
async fn send_delta_chunk(
    tx: &tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
    request_id: &Arc<String>,
    model_id: &Arc<String>,
    created: i64,
    content: String,
) -> bool {
    let chunk = create_chunk(
        request_id,
        model_id,
        created,
        None,
        Some(content),
        None,
    );
    
    send_chunk_event(tx, chunk).await
}

/// Send the final chunk with finish reason and DONE marker
async fn send_done_chunk(
    tx: &tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
    request_id: &Arc<String>,
    model_id: &Arc<String>,
    created: i64,
    finish_reason: chatsafe_common::FinishReason,
) -> bool {
    // Send final chunk with finish reason
    let chunk = create_chunk(
        request_id,
        model_id,
        created,
        None,
        None,
        Some(finish_reason),
    );
    
    if !send_chunk_event(tx, chunk).await {
        return false;
    }
    
    // Send [DONE] marker
    tx.send(Ok(Event::default().data(DONE_MARKER))).await.is_ok()
}

/// Create a ChatCompletionChunk with the given parameters
fn create_chunk(
    request_id: &Arc<String>,
    model_id: &Arc<String>,
    created: i64,
    role: Option<chatsafe_common::Role>,
    content: Option<String>,
    finish_reason: Option<chatsafe_common::FinishReason>,
) -> ChatCompletionChunk {
    ChatCompletionChunk {
        id: request_id.to_string(),
        object: CHUNK_OBJECT_TYPE.to_string(),
        created,
        model: model_id.to_string(),
        choices: vec![StreamChoice {
            index: 0,
            delta: DeltaContent { role, content },
            finish_reason,
        }],
    }
}

/// Send a chunk as an SSE event
async fn send_chunk_event(
    tx: &tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
    chunk: ChatCompletionChunk,
) -> bool {
    match serde_json::to_string(&chunk) {
        Ok(json) => {
            tx.send(Ok(Event::default().data(json))).await.is_ok()
        }
        Err(e) => {
            error!("Failed to serialize chunk: {}", e);
            false
        }
    }
}

/// Send an error event
async fn send_error_event(
    tx: &tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
    message: String,
    error_type: &str,
) -> bool {
    let error_data = json!({
        "error": {
            "message": message,
            "type": error_type
        }
    });
    tx.send(Ok(Event::default().data(error_data.to_string()))).await.is_ok()
}

/// Get current Unix timestamp
fn get_unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// RAII guard for cleanup when streaming completes or is interrupted
struct CleanupGuard {
    rate_limiter: RateLimiter,
    client_ip: IpAddr,
    metrics: Arc<ObservableMetrics>,
    request_id: RequestId,
}

impl CleanupGuard {
    fn new(
        rate_limiter: RateLimiter,
        client_ip: IpAddr,
        metrics: Arc<ObservableMetrics>,
        request_id: RequestId,
    ) -> Self {
        Self { rate_limiter, client_ip, metrics, request_id }
    }
}

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        // Clone values needed for the spawned task
        let limiter = self.rate_limiter.clone();
        let ip = self.client_ip;
        let metrics = self.metrics.clone();
        let req_id = self.request_id.clone();
        
        // Spawn cleanup task
        tokio::spawn(async move {
            limiter.release_request(ip).await;
            metrics.complete_request(&req_id).await;
        });
    }
}