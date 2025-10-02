use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use chatsafe_common::{Error, Result};

// Constants
const CLEANUP_RETENTION_SECS: u64 = 300; // 5 minutes
const TOKENS_PER_MINUTE_TO_PER_SECOND: f64 = 60.0;

/// Token bucket implementation for rate limiting
#[derive(Debug, Clone)]
struct TokenBucket {
    /// Maximum number of tokens the bucket can hold
    capacity: u32,
    /// Current number of tokens available
    tokens: f64,
    /// Rate at which tokens refill (tokens per second)
    refill_rate: f64,
    /// Last time the bucket was refilled
    last_refill: Instant,
}

impl TokenBucket {
    fn new(capacity: u32, refill_rate: f64) -> Self {
        Self {
            capacity,
            tokens: capacity as f64,
            refill_rate,
            last_refill: Instant::now(),
        }
    }

    /// Try to consume tokens from the bucket
    /// Returns true if successful, false if not enough tokens
    fn try_consume(&mut self, tokens: u32) -> bool {
        self.refill();
        
        if self.tokens >= tokens as f64 {
            self.tokens -= tokens as f64;
            true
        } else {
            false
        }
    }

    /// Refill tokens based on elapsed time
    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        
        self.tokens = (self.tokens + elapsed * self.refill_rate)
            .min(self.capacity as f64);
        self.last_refill = now;
    }
    
    /// Return a consumed token (for rollback scenarios)
    fn return_token(&mut self) {
        self.tokens = (self.tokens + 1.0).min(self.capacity as f64);
    }
}

/// Rate limiter configuration
#[derive(Debug, Clone)]
pub struct RateLimiterConfig {
    /// Max requests per IP per minute
    pub per_ip_per_minute: u32,
    /// Max concurrent requests per IP
    pub max_concurrent_per_ip: usize,
    /// Global max requests per minute
    pub global_per_minute: u32,
    /// Cleanup interval for expired entries
    pub cleanup_interval: Duration,
}

impl Default for RateLimiterConfig {
    fn default() -> Self {
        Self {
            per_ip_per_minute: 60,
            max_concurrent_per_ip: 5,
            global_per_minute: 600,
            cleanup_interval: Duration::from_secs(60),
        }
    }
}

/// Per-IP rate limiting state
#[derive(Debug)]
struct IpState {
    /// Token bucket for this IP
    bucket: TokenBucket,
    /// Number of concurrent requests from this IP
    concurrent_requests: usize,
    /// Last time this IP made a request
    last_seen: Instant,
}

/// Rate limiter for the API
/// 
/// Implements both per-IP and global rate limiting using token bucket algorithm.
/// Also enforces concurrent request limits per IP.
pub struct RateLimiter {
    config: RateLimiterConfig,
    ip_states: Arc<RwLock<HashMap<IpAddr, IpState>>>,
    global_bucket: Arc<RwLock<TokenBucket>>,
    cleanup_handle: Arc<RwLock<Option<JoinHandle<()>>>>,
}

impl Clone for RateLimiter {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            ip_states: self.ip_states.clone(),
            global_bucket: self.global_bucket.clone(),
            cleanup_handle: self.cleanup_handle.clone(),
        }
    }
}

impl RateLimiter {
    /// Create a new rate limiter with the given configuration
    pub fn new(config: RateLimiterConfig) -> Self {
        let global_bucket = TokenBucket::new(
            config.global_per_minute,
            config.global_per_minute as f64 / TOKENS_PER_MINUTE_TO_PER_SECOND,
        );

        let limiter = Self {
            config,
            ip_states: Arc::new(RwLock::new(HashMap::new())),
            global_bucket: Arc::new(RwLock::new(global_bucket)),
            cleanup_handle: Arc::new(RwLock::new(None)),
        };

        // Start cleanup task
        limiter.start_cleanup_task();
        limiter
    }
    
    /// Start the background cleanup task
    fn start_cleanup_task(&self) {
        let ip_states = self.ip_states.clone();
        let interval = self.config.cleanup_interval;
        
        let handle = tokio::spawn(async move {
            Self::cleanup_loop(ip_states, interval).await;
        });
        
        // Store handle for cleanup
        let cleanup_handle = self.cleanup_handle.clone();
        tokio::spawn(async move {
            let mut guard = cleanup_handle.write().await;
            *guard = Some(handle);
        });
    }

    /// Check if a request from the given IP is allowed
    /// 
    /// This checks both global and per-IP rate limits, as well as
    /// concurrent request limits. Returns an error if any limit is exceeded.
    /// Optimized to avoid double-locking when global check fails.
    pub async fn check_rate_limit(&self, ip: IpAddr) -> Result<()> {
        // Structure to hold rollback info if needed
        struct RollbackInfo {
            ip: IpAddr,
            needs_rollback: bool,
        }
        
        let mut rollback_info = RollbackInfo {
            ip,
            needs_rollback: false,
        };
        
        // First check and update per-IP limits
        {
            let mut states = self.ip_states.write().await;
            let state = states.entry(ip).or_insert_with(|| {
                IpState {
                    bucket: TokenBucket::new(
                        self.config.per_ip_per_minute,
                        self.config.per_ip_per_minute as f64 / TOKENS_PER_MINUTE_TO_PER_SECOND,
                    ),
                    concurrent_requests: 0,
                    last_seen: Instant::now(),
                }
            });

            // Update last seen
            state.last_seen = Instant::now();

            // Check concurrent request limit
            if state.concurrent_requests >= self.config.max_concurrent_per_ip {
                return Err(Error::RateLimitExceeded);
            }

            // Check token bucket
            if !state.bucket.try_consume(1) {
                return Err(Error::RateLimitExceeded);
            }
            
            // Update state - will need rollback if global check fails
            state.concurrent_requests += 1;
            rollback_info.needs_rollback = true;
        } // Lock released here
        
        // Then check global rate limit
        let global_check_passed = {
            let mut global_bucket = self.global_bucket.write().await;
            global_bucket.try_consume(1)
        };
        
        if !global_check_passed {
            // Rollback IP state if needed
            if rollback_info.needs_rollback {
                let mut states = self.ip_states.write().await;
                if let Some(state) = states.get_mut(&rollback_info.ip) {
                    state.concurrent_requests = state.concurrent_requests.saturating_sub(1);
                    state.bucket.return_token();
                }
            }
            return Err(Error::RateLimitExceeded);
        }

        Ok(())
    }

    /// Mark a request as completed, releasing the concurrent request slot
    pub async fn release_request(&self, ip: IpAddr) {
        let mut states = self.ip_states.write().await;
        if let Some(state) = states.get_mut(&ip) {
            state.concurrent_requests = state.concurrent_requests.saturating_sub(1);
        }
    }

    /// Background cleanup loop to remove old IP entries
    async fn cleanup_loop(
        ip_states: Arc<RwLock<HashMap<IpAddr, IpState>>>,
        cleanup_interval: Duration,
    ) {
        let mut interval = tokio::time::interval(cleanup_interval);
        loop {
            interval.tick().await;
            
            let mut states = ip_states.write().await;
            let now = Instant::now();
            
            // Remove IPs that haven't been seen for the retention period
            // and have no concurrent requests
            states.retain(|_, state| {
                now.duration_since(state.last_seen) < Duration::from_secs(CLEANUP_RETENTION_SECS)
                    || state.concurrent_requests > 0
            });
        }
    }
    
    /// Stop the cleanup task (for graceful shutdown)
    #[allow(dead_code)]
    pub async fn shutdown(&self) {
        if let Some(handle) = self.cleanup_handle.write().await.take() {
            handle.abort();
        }
    }
}

impl Drop for RateLimiter {
    fn drop(&mut self) {
        // Attempt to stop cleanup task on drop
        // Note: We can't await in drop, so we spawn a detached task
        let cleanup_handle = self.cleanup_handle.clone();
        tokio::spawn(async move {
            if let Some(handle) = cleanup_handle.write().await.take() {
                handle.abort();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[tokio::test]
    async fn test_rate_limiting() {
        let config = RateLimiterConfig {
            per_ip_per_minute: 10,
            max_concurrent_per_ip: 2,
            global_per_minute: 100,
            cleanup_interval: Duration::from_secs(60),
        };
        
        let limiter = RateLimiter::new(config);
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        // First 10 requests should succeed
        for _ in 0..10 {
            assert!(limiter.check_rate_limit(ip).await.is_ok());
            limiter.release_request(ip).await;
        }

        // 11th request should fail
        assert!(limiter.check_rate_limit(ip).await.is_err());
    }

    #[tokio::test]
    async fn test_concurrent_limit() {
        let config = RateLimiterConfig {
            per_ip_per_minute: 100,
            max_concurrent_per_ip: 2,
            global_per_minute: 1000,
            cleanup_interval: Duration::from_secs(60),
        };
        
        let limiter = RateLimiter::new(config);
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        // First 2 concurrent requests should succeed
        assert!(limiter.check_rate_limit(ip).await.is_ok());
        assert!(limiter.check_rate_limit(ip).await.is_ok());

        // 3rd concurrent request should fail
        assert!(limiter.check_rate_limit(ip).await.is_err());

        // Release one request
        limiter.release_request(ip).await;

        // Now another request should succeed
        assert!(limiter.check_rate_limit(ip).await.is_ok());
    }
    
    #[tokio::test]
    async fn test_global_limit_rollback() {
        let config = RateLimiterConfig {
            per_ip_per_minute: 100,
            max_concurrent_per_ip: 10,
            global_per_minute: 1,  // Very low global limit
            cleanup_interval: Duration::from_secs(60),
        };
        
        let limiter = RateLimiter::new(config);
        let ip1 = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let ip2 = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2));

        // First request should succeed
        assert!(limiter.check_rate_limit(ip1).await.is_ok());
        
        // Second request from different IP should fail (global limit)
        assert!(limiter.check_rate_limit(ip2).await.is_err());
        
        // IP2 state should be rolled back (no concurrent requests)
        {
            let states = limiter.ip_states.read().await;
            if let Some(state) = states.get(&ip2) {
                assert_eq!(state.concurrent_requests, 0);
            }
        }
    }
}