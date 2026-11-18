//! Retries (design.md 8.4) with the defaults of every runtime.

use crate::HttpResponse;
use std::time::Duration;

/// Retry options. Idempotent methods, network failures and the statuses 408, 429, 500, 502, 503 and
/// 504, up to 2 retries; the wait is `Retry-After` when the response has it, otherwise exponential
/// backoff with full jitter, capped by `max_delay`. Timeouts are not retried, and a wait that would
/// end after the client's timeout is not started.
#[derive(Debug, Clone)]
pub struct Retry {
    pub max: u32,
    pub methods: Vec<String>,
    pub statuses: Vec<u16>,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for Retry {
    fn default() -> Self {
        Retry {
            max: 2,
            methods: ["GET", "HEAD", "OPTIONS", "PUT", "DELETE"]
                .map(str::to_owned)
                .to_vec(),
            statuses: vec![408, 429, 500, 502, 503, 504],
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
        }
    }
}

impl Retry {
    pub(crate) fn retries(&self, method: &str) -> u32 {
        if self.methods.iter().any(|m| m.eq_ignore_ascii_case(method)) {
            self.max
        } else {
            0
        }
    }

    /// The wait before retry number `attempt` (from 1).
    pub fn delay(&self, attempt: u32, response: Option<&HttpResponse>) -> Duration {
        if let Some(after) = response.and_then(retry_after) {
            return after.min(self.max_delay);
        }
        let ceiling = self
            .base_delay
            .saturating_mul(2u32.saturating_pow(attempt - 1))
            .min(self.max_delay);
        ceiling.mul_f64(fastrand::f64())
    }
}

/// `Retry-After`: seconds or an HTTP date.
pub fn retry_after(response: &HttpResponse) -> Option<Duration> {
    let value = response.header("retry-after")?.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let at = httpdate::parse_http_date(value).ok()?;
    Some(
        at.duration_since(std::time::SystemTime::now())
            .unwrap_or_default(),
    )
}
