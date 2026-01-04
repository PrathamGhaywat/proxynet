use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct MemoryCache {
    data: Arc<RwLock<HashMap<String, (String, Instant)>>>,
}

impl MemoryCache {
    pub fn new() -> Self {
        Self {
            data: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn get(&self, key: &str) -> Option<String> {
        let cache = self.data.read().await;
        if let Some((value, expires_at)) = cache.get(key) {
            if Instant::now() < *expires_at {
                return Some(value.clone());
            } else {
                //remove expired entry
                drop(cache);
                let mut cache = self.data.write().await;
                cache.remove(key);
            }
        }
        None
    }

    pub async fn set(&self, key: String, value: String, ttl_seconds: u64) {
        let expires_at = Instant::now() + Duration::from_secs(ttl_seconds);
        let mut cache = self.data.write().await;
        cache.insert(key, (value, expires_at));
    }

    pub fn generate_cache_key(domain: &str, path: &str, query: Option<&str>) -> String {
        let query_part = query.map(|q| format!("?{}", q)).unwrap_or_default();
        format!("cache:{}:{}{}", domain, path, query_part)
    }
}