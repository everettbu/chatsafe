use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use serde::Serialize;

/// Privacy-preserving metrics collection - no PII, no payloads
#[derive(Debug, Clone)]
pub struct Metrics {
    inner: Arc<RwLock<MetricsInner>>,
    start_time: Instant,
}

#[derive(Debug)]
struct MetricsInner {
    // Request counters
    total_requests: u64,
    streaming_requests: u64,
    non_streaming_requests: u64,
    
    // Performance metrics
    total_prompt_tokens: u64,
    total_completion_tokens: u64,
    total_chunks_sent: u64,
    
    // Latency tracking (in milliseconds)
    first_token_latencies: Vec<u64>,
    request_durations: Vec<u64>,
    
    // Error tracking by type
    errors_by_type: HashMap<String, u64>,
    
    // Cancellation tracking
    cancelled_requests: u64,
    
    // Model usage (no PII - just counts)
    requests_by_model: HashMap<String, u64>,
    
    // Rate tracking
    tokens_per_second_samples: Vec<f64>,
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(MetricsInner {
                total_requests: 0,
                streaming_requests: 0,
                non_streaming_requests: 0,
                total_prompt_tokens: 0,
                total_completion_tokens: 0,
                total_chunks_sent: 0,
                first_token_latencies: Vec::new(),
                request_durations: Vec::new(),
                errors_by_type: HashMap::new(),
                cancelled_requests: 0,
                requests_by_model: HashMap::new(),
                tokens_per_second_samples: Vec::new(),
            })),
            start_time: Instant::now(),
        }
    }
    
    /// Record a new request (no content logged)
    pub async fn record_request(&self, model_id: &str, is_streaming: bool) {
        let mut metrics = self.inner.write().await;
        metrics.total_requests += 1;
        
        if is_streaming {
            metrics.streaming_requests += 1;
        } else {
            metrics.non_streaming_requests += 1;
        }
        
        *metrics.requests_by_model.entry(model_id.to_string()).or_insert(0) += 1;
    }
    
    /// Record token counts (no content)
    pub async fn record_tokens(&self, prompt_tokens: u64, completion_tokens: u64) {
        let mut metrics = self.inner.write().await;
        metrics.total_prompt_tokens += prompt_tokens;
        metrics.total_completion_tokens += completion_tokens;
    }
    
    /// Record first token latency
    pub async fn record_first_token_latency(&self, latency_ms: u64) {
        let mut metrics = self.inner.write().await;
        metrics.first_token_latencies.push(latency_ms);
        
        // Keep only last 1000 samples to prevent unbounded growth
        if metrics.first_token_latencies.len() > 1000 {
            metrics.first_token_latencies.remove(0);
        }
    }
    
    /// Record request duration
    pub async fn record_request_duration(&self, duration_ms: u64) {
        let mut metrics = self.inner.write().await;
        metrics.request_durations.push(duration_ms);
        
        // Keep only last 1000 samples
        if metrics.request_durations.len() > 1000 {
            metrics.request_durations.remove(0);
        }
    }
    
    /// Record streaming chunk sent
    pub async fn record_chunk_sent(&self) {
        let mut metrics = self.inner.write().await;
        metrics.total_chunks_sent += 1;
    }
    
    /// Record tokens per second sample
    pub async fn record_tokens_per_second(&self, tps: f64) {
        let mut metrics = self.inner.write().await;
        metrics.tokens_per_second_samples.push(tps);
        
        // Keep only last 100 samples
        if metrics.tokens_per_second_samples.len() > 100 {
            metrics.tokens_per_second_samples.remove(0);
        }
    }
    
    /// Record an error by type (no details)
    pub async fn record_error(&self, error_type: &str) {
        let mut metrics = self.inner.write().await;
        *metrics.errors_by_type.entry(error_type.to_string()).or_insert(0) += 1;
    }
    
    /// Record a cancelled request
    pub async fn record_cancellation(&self) {
        let mut metrics = self.inner.write().await;
        metrics.cancelled_requests += 1;
    }
    
    /// Get current metrics snapshot (for /metrics endpoint)
    pub async fn get_snapshot(&self) -> MetricsSnapshot {
        let metrics = self.inner.read().await;
        let uptime_seconds = self.start_time.elapsed().as_secs();
        
        // Calculate percentiles for latencies
        let p50_first_token = calculate_percentile(&metrics.first_token_latencies, 50);
        let p90_first_token = calculate_percentile(&metrics.first_token_latencies, 90);
        let p99_first_token = calculate_percentile(&metrics.first_token_latencies, 99);
        
        let p50_duration = calculate_percentile(&metrics.request_durations, 50);
        let p90_duration = calculate_percentile(&metrics.request_durations, 90);
        let p99_duration = calculate_percentile(&metrics.request_durations, 99);
        
        // Calculate average tokens per second
        let avg_tps = if !metrics.tokens_per_second_samples.is_empty() {
            metrics.tokens_per_second_samples.iter().sum::<f64>() 
                / metrics.tokens_per_second_samples.len() as f64
        } else {
            0.0
        };
        
        MetricsSnapshot {
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            uptime_seconds,
            
            // Counts
            total_requests: metrics.total_requests,
            streaming_requests: metrics.streaming_requests,
            non_streaming_requests: metrics.non_streaming_requests,
            cancelled_requests: metrics.cancelled_requests,
            total_errors: metrics.errors_by_type.values().sum(),
            
            // Tokens
            total_prompt_tokens: metrics.total_prompt_tokens,
            total_completion_tokens: metrics.total_completion_tokens,
            total_chunks_sent: metrics.total_chunks_sent,
            
            // Performance
            avg_tokens_per_second: avg_tps,
            p50_first_token_ms: p50_first_token,
            p90_first_token_ms: p90_first_token,
            p99_first_token_ms: p99_first_token,
            p50_request_duration_ms: p50_duration,
            p90_request_duration_ms: p90_duration,
            p99_request_duration_ms: p99_duration,
            
            // Error breakdown (no details, just counts)
            errors_by_type: metrics.errors_by_type.clone(),
            
            // Model usage (no PII)
            requests_by_model: metrics.requests_by_model.clone(),
        }
    }
}

/// Metrics snapshot for external consumption
#[derive(Debug, Clone, Serialize)]
pub struct MetricsSnapshot {
    pub timestamp: u64,
    pub uptime_seconds: u64,
    
    // Request counts
    pub total_requests: u64,
    pub streaming_requests: u64,
    pub non_streaming_requests: u64,
    pub cancelled_requests: u64,
    pub total_errors: u64,
    
    // Token counts
    pub total_prompt_tokens: u64,
    pub total_completion_tokens: u64,
    pub total_chunks_sent: u64,
    
    // Performance metrics
    pub avg_tokens_per_second: f64,
    pub p50_first_token_ms: u64,
    pub p90_first_token_ms: u64,
    pub p99_first_token_ms: u64,
    pub p50_request_duration_ms: u64,
    pub p90_request_duration_ms: u64,
    pub p99_request_duration_ms: u64,
    
    // Error breakdown
    pub errors_by_type: HashMap<String, u64>,
    
    // Model usage
    pub requests_by_model: HashMap<String, u64>,
}

/// Calculate percentile from sorted samples
fn calculate_percentile(samples: &[u64], percentile: usize) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    
    let index = (sorted.len() - 1) * percentile / 100;
    sorted[index]
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}