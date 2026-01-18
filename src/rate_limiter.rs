use std::{collections::HashMap, sync::Arc, time::{Duration, Instant}};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<Mutex<HashMap<String, (u32, Instant)>>>,
    window: Duration,
    limit: u32,
}

impl RateLimiter {
    pub fn new(limit: u32, window_seconds: u64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            window: Duration::from_secs(window_seconds),
            limit,
        }
    }

    pub async fn allow(&self, key: &str) -> bool {
        let mut map = self.inner.lock().await;
        let now = Instant::now();

        match map.get_mut(key) {
            Some((count, start)) => {
                if now.duration_since(*start) > self.window {
                    *count = 1;
                    *start = now;
                    true
                } else {
                    if *count < self.limit {
                        *count += 1;
                        true
                    } else {
                        false
                    }
                }
            }
            None => {
                map.insert(key.to_string(), (1, now));
                true
            }
        }
    }

    pub fn spawn_cleanup(&self) {
        let inner = self.inner.clone();
        let window = self.window;
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(window).await;
                let mut map = inner.lock().await;
                let now = Instant::now();
                let max_age = window + window;
                map.retain(|_, (_, start)| now.duration_since(*start) <= max_age);
            }
        });
    }
}