use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use chatsafe_common::{Error, Result};

/// Token bucket implementation for rate limiting
#[derive(Debug, Clone)]
struct TokenBucket {
    capacity: u32,
    tokens: f64,
    refill_rate: f64,  // tokens per second
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

    fn try_consume(&mut self, tokens: u32) -> bool {
        self.refill();
        
        if self.tokens >= tokens as f64 {
            self.tokens -= tokens as f64;
            true
        } else {
            false
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        
        self.tokens = (self.tokens + elapsed * self.refill_rate)
            .min(self.capacity as f64);
        self.last_refill = now;
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
    bucket: TokenBucket,
    concurrent_requests: usize,
    last_seen: Instant,
}

/// Rate limiter for the API
#[derive(Clone)]
pub struct RateLimiter {
    config: RateLimiterConfig,
    ip_states: Arc<RwLock<HashMap<IpAddr, IpState>>>,
    global_bucket: Arc<RwLock<TokenBucket>>,
}

impl RateLimiter {
    pub fn new(config: RateLimiterConfig) -> Self {
        let global_bucket = TokenBucket::new(
            config.global_per_minute,
            config.global_per_minute as f64 / 60.0,
        );

        let limiter = Self {
            config: config.clone(),
            ip_states: Arc::new(RwLock::new(HashMap::new())),
            global_bucket: Arc::new(RwLock::new(global_bucket)),
        };

        // Spawn cleanup task
        let limiter_clone = limiter.clone();
        tokio::spawn(async move {
            limiter_clone.cleanup_loop().await;
        });

        limiter
    }

    /// Check if a request from the given IP is allowed
    pub async fn check_rate_limit(&self, ip: IpAddr) -> Result<()> {
        // Check global rate limit first
        {
            let mut global_bucket = self.global_bucket.write().await;
            if !global_bucket.try_consume(1) {
                return Err(Error::RateLimitExceeded);
            }
        }

        // Check per-IP rate limit
        let mut states = self.ip_states.write().await;
        let state = states.entry(ip).or_insert_with(|| {
            IpState {
                bucket: TokenBucket::new(
                    self.config.per_ip_per_minute,
                    self.config.per_ip_per_minute as f64 / 60.0,
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

        state.concurrent_requests += 1;
        Ok(())
    }

    /// Mark a request as completed
    pub async fn release_request(&self, ip: IpAddr) {
        let mut states = self.ip_states.write().await;
        if let Some(state) = states.get_mut(&ip) {
            state.concurrent_requests = state.concurrent_requests.saturating_sub(1);
        }
    }

    /// Get current limits for an IP
    pub async fn get_limits(&self, ip: IpAddr) -> (u32, usize) {
        let states = self.ip_states.read().await;
        if let Some(state) = states.get(&ip) {
            (
                state.bucket.tokens as u32,
                self.config.max_concurrent_per_ip - state.concurrent_requests,
            )
        } else {
            (
                self.config.per_ip_per_minute,
                self.config.max_concurrent_per_ip,
            )
        }
    }

    /// Cleanup old entries
    async fn cleanup_loop(&self) {
        let mut interval = tokio::time::interval(self.config.cleanup_interval);
        loop {
            interval.tick().await;
            
            let mut states = self.ip_states.write().await;
            let now = Instant::now();
            
            // Remove IPs that haven't been seen for 5 minutes
            states.retain(|_, state| {
                now.duration_since(state.last_seen) < Duration::from_secs(300)
                    || state.concurrent_requests > 0
            });
        }
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
}