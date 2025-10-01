use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use serde::Serialize;
use uuid::Uuid;

const MAX_SAMPLES: usize = 10000;
const PERCENTILE_WINDOW: usize = 1000;

/// Request correlation ID for tracing
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RequestId(Arc<str>);

impl Serialize for RequestId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl RequestId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string().into())
    }
    
    pub fn from_string(s: String) -> Self {
        Self(s.into())
    }
    
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Error categories for metrics
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum ErrorCategory {
    BadRequest,      // 400 - Client error
    RateLimited,     // 429 - Too many requests
    Timeout,         // 408/504 - Request timeout
    Cancelled,       // 499 - Client cancelled
    Unavailable,     // 503 - Service unavailable
    Internal,        // 500 - Internal error
}

impl ErrorCategory {
    pub fn from_error(error: &crate::Error) -> Self {
        match error {
            crate::Error::BadRequest(_) |
            crate::Error::ValidationFailed(_) |
            crate::Error::InvalidModel(_) => ErrorCategory::BadRequest,
            
            crate::Error::RateLimitExceeded => ErrorCategory::RateLimited,
            
            crate::Error::Timeout(_) => ErrorCategory::Timeout,
            
            crate::Error::Cancelled(_) |
            crate::Error::UserCancelled => ErrorCategory::Cancelled,
            
            crate::Error::ServiceUnavailable(_) |
            crate::Error::ModelLoadFailed(_) |
            crate::Error::RuntimeNotReady |
            crate::Error::ModelNotFound(_) => ErrorCategory::Unavailable,
            
            _ => ErrorCategory::Internal,
        }
    }
    
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorCategory::BadRequest => "bad_request",
            ErrorCategory::RateLimited => "rate_limited",
            ErrorCategory::Timeout => "timeout",
            ErrorCategory::Cancelled => "cancelled",
            ErrorCategory::Unavailable => "unavailable",
            ErrorCategory::Internal => "internal",
        }
    }
}

/// Active request tracking
#[derive(Debug, Clone)]
pub struct ActiveRequest {
    pub request_id: RequestId,
    pub started_at: Instant,
    pub model: String,
    pub is_streaming: bool,
}

/// Enhanced metrics with observability
#[derive(Debug, Clone)]
pub struct ObservableMetrics {
    inner: Arc<RwLock<MetricsData>>,
    start_time: Instant,
}

#[derive(Debug)]
struct MetricsData {
    // Request tracking
    total_requests: u64,
    streaming_requests: u64,
    non_streaming_requests: u64,
    active_requests: HashMap<RequestId, ActiveRequest>,
    
    // Performance metrics with percentiles
    first_token_latencies: VecDeque<u64>,
    request_durations: VecDeque<u64>,
    tokens_per_second: VecDeque<f64>,
    
    // Token tracking
    total_prompt_tokens: u64,
    total_completion_tokens: u64,
    total_chunks_sent: u64,
    
    // Error tracking by category
    errors_by_category: HashMap<ErrorCategory, u64>,
    error_messages: VecDeque<(Instant, ErrorCategory, String)>,
    
    // Cancellation and timeout tracking
    cancelled_requests: u64,
    timed_out_requests: u64,
    
    // Rate limiting metrics
    rate_limit_hits: u64,
    rate_limit_by_ip: HashMap<String, u64>,
    
    // Model usage
    requests_by_model: HashMap<String, u64>,
    
    // Stream metrics
    active_streams: u64,
    completed_streams: u64,
    failed_streams: u64,
}

impl ObservableMetrics {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(MetricsData {
                total_requests: 0,
                streaming_requests: 0,
                non_streaming_requests: 0,
                active_requests: HashMap::new(),
                first_token_latencies: VecDeque::new(),
                request_durations: VecDeque::new(),
                tokens_per_second: VecDeque::new(),
                total_prompt_tokens: 0,
                total_completion_tokens: 0,
                total_chunks_sent: 0,
                errors_by_category: HashMap::new(),
                error_messages: VecDeque::new(),
                cancelled_requests: 0,
                timed_out_requests: 0,
                rate_limit_hits: 0,
                rate_limit_by_ip: HashMap::new(),
                requests_by_model: HashMap::new(),
                active_streams: 0,
                completed_streams: 0,
                failed_streams: 0,
            })),
            start_time: Instant::now(),
        }
    }
    
    /// Start tracking a request
    pub async fn start_request(&self, request_id: RequestId, model: String, is_streaming: bool) -> RequestId {
        let mut data = self.inner.write().await;
        
        data.total_requests += 1;
        if is_streaming {
            data.streaming_requests += 1;
            data.active_streams += 1;
        } else {
            data.non_streaming_requests += 1;
        }
        
        *data.requests_by_model.entry(model.clone()).or_insert(0) += 1;
        
        data.active_requests.insert(request_id.clone(), ActiveRequest {
            request_id: request_id.clone(),
            started_at: Instant::now(),
            model,
            is_streaming,
        });
        
        request_id
    }
    
    /// Complete a request
    pub async fn complete_request(&self, request_id: &RequestId) {
        let mut data = self.inner.write().await;
        
        if let Some(request) = data.active_requests.remove(request_id) {
            let duration_ms = request.started_at.elapsed().as_millis() as u64;
            
            data.request_durations.push_back(duration_ms);
            if data.request_durations.len() > MAX_SAMPLES {
                data.request_durations.pop_front();
            }
            
            if request.is_streaming {
                data.active_streams = data.active_streams.saturating_sub(1);
                data.completed_streams += 1;
            }
        }
    }
    
    /// Record an error with category
    pub async fn record_error(&self, request_id: Option<&RequestId>, error: &crate::Error) {
        let mut data = self.inner.write().await;
        
        let category = ErrorCategory::from_error(error);
        *data.errors_by_category.entry(category).or_insert(0) += 1;
        
        // Store error message for recent errors (last 100)
        data.error_messages.push_back((Instant::now(), category, error.to_string()));
        if data.error_messages.len() > 100 {
            data.error_messages.pop_front();
        }
        
        // Update specific counters
        match category {
            ErrorCategory::Cancelled => data.cancelled_requests += 1,
            ErrorCategory::Timeout => data.timed_out_requests += 1,
            ErrorCategory::RateLimited => data.rate_limit_hits += 1,
            _ => {}
        }
        
        // Mark stream as failed if applicable
        if let Some(id) = request_id {
            if let Some(request) = data.active_requests.remove(id) {
                if request.is_streaming {
                    data.active_streams = data.active_streams.saturating_sub(1);
                    data.failed_streams += 1;
                }
            }
        }
    }
    
    /// Record rate limit hit for an IP
    pub async fn record_rate_limit(&self, ip: String) {
        let mut data = self.inner.write().await;
        data.rate_limit_hits += 1;
        *data.rate_limit_by_ip.entry(ip).or_insert(0) += 1;
    }
    
    /// Record first token latency
    pub async fn record_first_token_latency(&self, latency_ms: u64) {
        let mut data = self.inner.write().await;
        data.first_token_latencies.push_back(latency_ms);
        if data.first_token_latencies.len() > MAX_SAMPLES {
            data.first_token_latencies.pop_front();
        }
    }
    
    /// Record tokens per second
    pub async fn record_tokens_per_second(&self, tps: f64) {
        let mut data = self.inner.write().await;
        data.tokens_per_second.push_back(tps);
        if data.tokens_per_second.len() > MAX_SAMPLES {
            data.tokens_per_second.pop_front();
        }
    }
    
    /// Record token counts
    pub async fn record_tokens(&self, prompt: u64, completion: u64) {
        let mut data = self.inner.write().await;
        data.total_prompt_tokens += prompt;
        data.total_completion_tokens += completion;
    }
    
    /// Record chunk sent
    pub async fn record_chunk(&self) {
        let mut data = self.inner.write().await;
        data.total_chunks_sent += 1;
    }
    
    /// Calculate percentile from samples
    fn calculate_percentile(samples: &[u64], percentile: f64) -> u64 {
        if samples.is_empty() {
            return 0;
        }
        
        let mut sorted = samples.to_vec();
        sorted.sort_unstable();
        
        let index = ((percentile / 100.0) * (sorted.len() - 1) as f64) as usize;
        sorted[index]
    }
    
    /// Get detailed metrics snapshot
    pub async fn snapshot(&self) -> MetricsSnapshot {
        let data = self.inner.read().await;
        
        // Calculate percentiles
        let first_token_samples: Vec<u64> = data.first_token_latencies.iter().copied().collect();
        let duration_samples: Vec<u64> = data.request_durations.iter().copied().collect();
        let tps_samples: Vec<f64> = data.tokens_per_second.iter().copied().collect();
        
        let first_token_p50 = Self::calculate_percentile(&first_token_samples, 50.0);
        let first_token_p95 = Self::calculate_percentile(&first_token_samples, 95.0);
        let first_token_p99 = Self::calculate_percentile(&first_token_samples, 99.0);
        
        let duration_p50 = Self::calculate_percentile(&duration_samples, 50.0);
        let duration_p95 = Self::calculate_percentile(&duration_samples, 95.0);
        let duration_p99 = Self::calculate_percentile(&duration_samples, 99.0);
        
        let avg_tps = if !tps_samples.is_empty() {
            tps_samples.iter().sum::<f64>() / tps_samples.len() as f64
        } else {
            0.0
        };
        
        MetricsSnapshot {
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            uptime_seconds: self.start_time.elapsed().as_secs(),
            
            // Request counts
            total_requests: data.total_requests,
            streaming_requests: data.streaming_requests,
            non_streaming_requests: data.non_streaming_requests,
            active_requests: data.active_requests.len() as u64,
            
            // Latency percentiles
            first_token_latency_p50_ms: first_token_p50,
            first_token_latency_p95_ms: first_token_p95,
            first_token_latency_p99_ms: first_token_p99,
            
            request_duration_p50_ms: duration_p50,
            request_duration_p95_ms: duration_p95,
            request_duration_p99_ms: duration_p99,
            
            // Token metrics
            total_prompt_tokens: data.total_prompt_tokens,
            total_completion_tokens: data.total_completion_tokens,
            total_chunks_sent: data.total_chunks_sent,
            average_tokens_per_second: avg_tps,
            
            // Stream metrics
            active_streams: data.active_streams,
            completed_streams: data.completed_streams,
            failed_streams: data.failed_streams,
            
            // Error metrics
            errors_by_category: data.errors_by_category.clone(),
            cancelled_requests: data.cancelled_requests,
            timed_out_requests: data.timed_out_requests,
            
            // Rate limiting
            rate_limit_hits: data.rate_limit_hits,
            
            // Model usage
            requests_by_model: data.requests_by_model.clone(),
        }
    }
    
    /// Get recent errors for debugging
    pub async fn recent_errors(&self) -> Vec<(u64, String, String)> {
        let data = self.inner.read().await;
        let now = Instant::now();
        
        data.error_messages
            .iter()
            .map(|(time, category, msg)| {
                let age_seconds = now.duration_since(*time).as_secs();
                (age_seconds, category.as_str().to_string(), msg.clone())
            })
            .collect()
    }
}

/// Metrics snapshot for /metrics endpoint
#[derive(Debug, Clone, Serialize)]
pub struct MetricsSnapshot {
    pub timestamp: u64,
    pub uptime_seconds: u64,
    
    // Request counts
    pub total_requests: u64,
    pub streaming_requests: u64,
    pub non_streaming_requests: u64,
    pub active_requests: u64,
    
    // Latency percentiles (milliseconds)
    pub first_token_latency_p50_ms: u64,
    pub first_token_latency_p95_ms: u64,
    pub first_token_latency_p99_ms: u64,
    
    pub request_duration_p50_ms: u64,
    pub request_duration_p95_ms: u64,
    pub request_duration_p99_ms: u64,
    
    // Token metrics
    pub total_prompt_tokens: u64,
    pub total_completion_tokens: u64,
    pub total_chunks_sent: u64,
    pub average_tokens_per_second: f64,
    
    // Stream metrics
    pub active_streams: u64,
    pub completed_streams: u64,
    pub failed_streams: u64,
    
    // Error metrics by category
    pub errors_by_category: HashMap<ErrorCategory, u64>,
    pub cancelled_requests: u64,
    pub timed_out_requests: u64,
    
    // Rate limiting
    pub rate_limit_hits: u64,
    
    // Model usage
    pub requests_by_model: HashMap<String, u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_request_tracking() {
        let metrics = ObservableMetrics::new();
        
        let req_id = metrics.start_request(
            RequestId::new(),
            "test-model".to_string(),
            true
        ).await;
        
        let snapshot = metrics.snapshot().await;
        assert_eq!(snapshot.total_requests, 1);
        assert_eq!(snapshot.active_requests, 1);
        assert_eq!(snapshot.active_streams, 1);
        
        metrics.complete_request(&req_id).await;
        
        let snapshot = metrics.snapshot().await;
        assert_eq!(snapshot.active_requests, 0);
        assert_eq!(snapshot.active_streams, 0);
        assert_eq!(snapshot.completed_streams, 1);
    }
    
    #[tokio::test]
    async fn test_error_categorization() {
        let metrics = ObservableMetrics::new();
        
        let bad_request = crate::Error::BadRequest("test".into());
        let rate_limit = crate::Error::RateLimitExceeded;
        
        metrics.record_error(None, &bad_request).await;
        metrics.record_error(None, &rate_limit).await;
        
        let snapshot = metrics.snapshot().await;
        assert_eq!(snapshot.errors_by_category.get(&ErrorCategory::BadRequest), Some(&1));
        assert_eq!(snapshot.errors_by_category.get(&ErrorCategory::RateLimited), Some(&1));
        assert_eq!(snapshot.rate_limit_hits, 1);
    }
    
    #[test]
    fn test_percentile_calculation() {
        let samples = vec![10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
        
        assert_eq!(ObservableMetrics::calculate_percentile(&samples, 50.0), 50);
        assert_eq!(ObservableMetrics::calculate_percentile(&samples, 95.0), 90);  // 95% of 10 samples = index 8 = value 90
        assert_eq!(ObservableMetrics::calculate_percentile(&samples, 99.0), 90);  // 99% of 10 samples = index 8.9 = index 8 = value 90
    }
}