use chrono::Utc;
use std::time::Instant;
use tracing::info;

#[derive(Debug, Clone)]
pub struct RequestLog {
    pub domain: String,
    pub path: String,
    pub method: String,
    pub status: u16,
    pub response_time_ms: u128,
    pub bytes_sent: u64,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub referer: Option<String>,
    pub timestamp: chrono::DateTime<Utc>,
}

impl RequestLog {
    pub fn new(
        domain: String,
        path: String,
        method: String,
        status: u16,
        response_time: Instant,
    ) -> Self {
        Self {
            domain,
            path,
            method,
            status,
            response_time_ms: response_time.elapsed().as_millis(),
            bytes_sent: 0, // We'll calculate this later
            ip_address: None,
            user_agent: None,
            referer: None,
            timestamp: Utc::now(),
        }
    }

    pub fn with_ip(mut self, ip: String) -> Self {
        self.ip_address = Some(ip);
        self
    }

    pub fn with_user_agent(mut self, user_agent: String) -> Self {
        self.user_agent = Some(user_agent);
        self
    }

    pub fn with_referer(mut self, referer: String) -> Self {
        self.referer = Some(referer);
        self
    }

    pub fn with_bytes(mut self, bytes: u64) -> Self {
        self.bytes_sent = bytes;
        self
    }

    pub fn log(&self) {
        info!(
            "logs: {} {} {} - {} in {}ms | IP: {} | UA: {}",
            self.method,
            self.domain,
            self.path,
            self.status,
            self.response_time_ms,
            self.ip_address.as_deref().unwrap_or("unknown"),
            self.user_agent.as_deref().unwrap_or("unknown"),
        );
    }

    
}