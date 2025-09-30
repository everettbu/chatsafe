use async_trait::async_trait;

/// Trait for metrics collection
#[async_trait]
pub trait MetricsProvider: Send + Sync {
    /// Record a request
    async fn record_request(&self, model: &str, stream: bool);
    
    /// Record tokens generated
    async fn record_tokens(&self, prompt_tokens: usize, completion_tokens: usize);
    
    /// Record request duration
    async fn record_duration(&self, duration_ms: u64);
    
    /// Record an error
    async fn record_error(&self, error_type: &str);
}

/// No-op implementation for when metrics are disabled
pub struct NoOpMetrics;

#[async_trait]
impl MetricsProvider for NoOpMetrics {
    async fn record_request(&self, _model: &str, _stream: bool) {}
    async fn record_tokens(&self, _prompt_tokens: usize, _completion_tokens: usize) {}
    async fn record_duration(&self, _duration_ms: u64) {}
    async fn record_error(&self, _error_type: &str) {}
}