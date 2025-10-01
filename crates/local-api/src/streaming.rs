use axum::response::sse::{Event, Sse};
use chatsafe_common::{
    ChatCompletionChunk, StreamChoice, DeltaContent, StreamFrame, Role,
    Error as CommonError, Metrics,
};
use futures::stream::Stream;
use futures::StreamExt;
use serde_json::json;
use std::convert::Infallible;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::error;
use uuid::Uuid;

use crate::rate_limiter::RateLimiter;

const BUFFER_SIZE: usize = 32;  // Maximum chunks to buffer
const CHUNK_TIMEOUT: Duration = Duration::from_secs(30);  // Timeout per chunk

pub fn streaming_response_with_backpressure(
    stream: std::pin::Pin<Box<dyn Stream<Item = Result<StreamFrame, CommonError>> + Send>>,
    model_id: String,
    metrics: Arc<Metrics>,
    rate_limiter: RateLimiter,
    client_ip: IpAddr,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // Use bounded channel for backpressure
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(BUFFER_SIZE);
    
    // Spawn producer task with automatic cleanup
    tokio::spawn(async move {
        produce_stream_events(stream, model_id, metrics, tx, rate_limiter, client_ip).await;
    });
    
    // Consumer stream that yields from the bounded channel
    let response_stream = async_stream::stream! {
        while let Some(event) = rx.recv().await {
            yield event;
        }
    };
    
    Sse::new(response_stream)
}

async fn produce_stream_events(
    mut stream: std::pin::Pin<Box<dyn Stream<Item = Result<StreamFrame, CommonError>> + Send>>,
    model_id: String,
    metrics: Arc<Metrics>,
    tx: tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
    rate_limiter: RateLimiter,
    client_ip: IpAddr,
) {
    // Ensure cleanup happens when function exits
    let _cleanup = CleanupGuard::new(rate_limiter.clone(), client_ip);
    
    let request_id = Arc::new(Uuid::new_v4().to_string());
    let model_id = Arc::new(model_id);
    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_secs() as i64;
    
    let mut first_token_recorded = false;
    let stream_start = std::time::Instant::now();
    let mut chunk_count = 0u64;
    
    while let Some(frame_result) = tokio::time::timeout(
        CHUNK_TIMEOUT,
        stream.next()
    ).await.ok().flatten() {
        let event = match frame_result {
            Ok(StreamFrame::Start { role, .. }) => {
                // Send initial chunk with role
                let chunk = ChatCompletionChunk {
                    id: request_id.to_string(),
                    object: "chat.completion.chunk".to_string(),
                    created,
                    model: model_id.to_string(),
                    choices: vec![StreamChoice {
                        index: 0,
                        delta: DeltaContent {
                            role: Some(role),
                            content: None,
                        },
                        finish_reason: None,
                    }],
                };
                
                match serde_json::to_string(&chunk) {
                    Ok(json) => Some(Event::default().data(json)),
                    Err(e) => {
                        error!("Failed to serialize start chunk: {}", e);
                        None
                    }
                }
            }
            Ok(StreamFrame::Delta { content }) => {
                // Record first token latency
                if !first_token_recorded {
                    first_token_recorded = true;
                    let latency_ms = stream_start.elapsed().as_millis() as u64;
                    let m = metrics.clone();
                    tokio::spawn(async move {
                        m.record_first_token_latency(latency_ms).await;
                    });
                }
                
                chunk_count += 1;
                
                // Send content chunk
                let chunk = ChatCompletionChunk {
                    id: request_id.to_string(),
                    object: "chat.completion.chunk".to_string(),
                    created,
                    model: model_id.to_string(),
                    choices: vec![StreamChoice {
                        index: 0,
                        delta: DeltaContent {
                            role: None,
                            content: Some(content),
                        },
                        finish_reason: None,
                    }],
                };
                
                // Track chunks sent
                let m = metrics.clone();
                tokio::spawn(async move {
                    m.record_chunk_sent().await;
                });
                
                match serde_json::to_string(&chunk) {
                    Ok(json) => Some(Event::default().data(json)),
                    Err(e) => {
                        error!("Failed to serialize delta chunk: {}", e);
                        None
                    }
                }
            }
            Ok(StreamFrame::Done { finish_reason, .. }) => {
                // Send final chunk with finish reason
                let chunk = ChatCompletionChunk {
                    id: request_id.to_string(),
                    object: "chat.completion.chunk".to_string(),
                    created,
                    model: model_id.to_string(),
                    choices: vec![StreamChoice {
                        index: 0,
                        delta: DeltaContent {
                            role: None,
                            content: None,
                        },
                        finish_reason: Some(finish_reason),
                    }],
                };
                
                // Send final chunk and [DONE] marker
                if let Ok(json) = serde_json::to_string(&chunk) {
                    if tx.send(Ok(Event::default().data(json))).await.is_err() {
                        return; // Client disconnected
                    }
                }
                
                // Send [DONE] marker
                let _ = tx.send(Ok(Event::default().data("[DONE]"))).await;
                return;
            }
            Ok(StreamFrame::Error { message }) => {
                // Send error as data
                let error_data = json!({
                    "error": {
                        "message": message,
                        "type": "runtime_error"
                    }
                });
                let _ = tx.send(Ok(Event::default().data(error_data.to_string()))).await;
                return;
            }
            Err(e) => {
                // Send error as data
                let error_data = json!({
                    "error": {
                        "message": e.to_string(),
                        "type": "stream_error"
                    }
                });
                let _ = tx.send(Ok(Event::default().data(error_data.to_string()))).await;
                return;
            }
        };
        
        // Send event if we have one
        if let Some(event) = event {
            if tx.send(Ok(event)).await.is_err() {
                // Client disconnected, stop producing
                return;
            }
        }
    }
}

/// RAII guard for cleanup
struct CleanupGuard {
    rate_limiter: RateLimiter,
    client_ip: IpAddr,
}

impl CleanupGuard {
    fn new(rate_limiter: RateLimiter, client_ip: IpAddr) -> Self {
        Self { rate_limiter, client_ip }
    }
}

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        let limiter = self.rate_limiter.clone();
        let ip = self.client_ip;
        tokio::spawn(async move {
            limiter.release_request(ip).await;
        });
    }
}