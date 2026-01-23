use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DomainDto {
    pub id: Option<i32>,
    pub domain: String,
    pub origin: String,
    pub enabled: bool,
}

#[derive(Debug, Serialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

impl<T> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn err(error: String) -> ApiResponse<()> {
        ApiResponse {
            success: false,
            data: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct StatsResponse {
    pub total_requests: i64,
    pub cache_hits: i64,
    pub cache_hit_rate: f64,
    pub avg_response_time_ms: f64,
    pub total_bytes_sent: i64,
}

pub fn api_router(
    routes: Arc<RwLock<HashMap<String, String>>>,
    db: SqlitePool,
) -> Router {
    Router::new()
        .route("/domains", get(list_domains).post(create_domain))
        .route("/domains/{id}", get(get_domain).patch(update_domain).delete(delete_domain))
        .route("/stats", get(get_stats))
        .with_state((routes, db))
}

async fn list_domains(
    State((routes, _db)): State<(Arc<RwLock<HashMap<String, String>>>, SqlitePool)>,
) -> impl IntoResponse {
    let routes_lock = routes.read().await;
    let domains: Vec<DomainDto> = routes_lock
        .iter()
        .enumerate()
        .map(|(idx, (domain, origin))| DomainDto {
            id: Some(idx as i32),
            domain: domain.clone(),
            origin: origin.clone(),
            enabled: true,
        })
        .collect();

    Json(ApiResponse::ok(domains))
}

async fn create_domain(
    State((routes, _db)): State<(Arc<RwLock<HashMap<String, String>>>, SqlitePool)>,
    Json(payload): Json<DomainDto>,
) -> impl IntoResponse {
    let mut routes_lock = routes.write().await;
    routes_lock.insert(payload.domain.clone(), payload.origin.clone());

    (StatusCode::CREATED, Json(ApiResponse::ok(payload)))
}

async fn get_domain(
    State((routes, _db)): State<(Arc<RwLock<HashMap<String, String>>>, SqlitePool)>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    let routes_lock = routes.read().await;
    let domain = routes_lock
        .iter()
        .enumerate()
        .find(|(idx, _)| *idx == id as usize)
        .map(|(_, (domain, origin))| DomainDto {
            id: Some(id),
            domain: domain.clone(),
            origin: origin.clone(),
            enabled: true,
        });

    match domain {
        Some(d) => (StatusCode::OK, Json(ApiResponse::ok(d))).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::err("Domain not found".to_string())),
        )
            .into_response(),
    }
}

async fn update_domain(
    State((routes, _db)): State<(Arc<RwLock<HashMap<String, String>>>, SqlitePool)>,
    Path(id): Path<i32>,
    Json(payload): Json<DomainDto>,
) -> impl IntoResponse {
    let mut routes_lock = routes.write().await;

    let old_domain = routes_lock
        .iter()
        .enumerate()
        .find(|(idx, _)| *idx == id as usize)
        .map(|(_, (d, _))| d.clone());

    if let Some(old_domain) = old_domain {
        routes_lock.remove(&old_domain);
        routes_lock.insert(payload.domain.clone(), payload.origin.clone());
        (StatusCode::OK, Json(ApiResponse::ok(payload))).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::err("Domain not found".to_string())),
        )
            .into_response()
    }
}

async fn delete_domain(
    State((routes, _db)): State<(Arc<RwLock<HashMap<String, String>>>, SqlitePool)>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    let mut routes_lock = routes.write().await;

    let domain_to_delete = routes_lock
        .iter()
        .enumerate()
        .find(|(idx, _)| *idx == id as usize)
        .map(|(_, (d, _))| d.clone());

    if let Some(domain) = domain_to_delete {
        routes_lock.remove(&domain);
        (StatusCode::NO_CONTENT, "").into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::err("Domain not found".to_string())),
        )
            .into_response()
    }
}

async fn get_stats(
    State((_routes, db)): State<(Arc<RwLock<HashMap<String, String>>>, SqlitePool)>,
) -> impl IntoResponse {
    let total = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM request_logs")
        .fetch_one(&db)
        .await
        .unwrap_or(0);

    let cache_hits = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM request_logs WHERE status = 200 LIMIT 1"
    )
    .fetch_one(&db)
    .await
    .unwrap_or(0);

    let avg_time: Option<f64> = sqlx::query_scalar("SELECT AVG(response_time_ms) FROM request_logs")
        .fetch_optional(&db)
        .await
        .unwrap_or(None);

    let total_bytes: i64 = sqlx::query_scalar("SELECT SUM(bytes_sent) FROM request_logs")
        .fetch_one(&db)
        .await
        .unwrap_or(0);

    let cache_hit_rate = if total > 0 {
        (cache_hits as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    let stats = StatsResponse {
        total_requests: total,
        cache_hits,
        cache_hit_rate,
        avg_response_time_ms: avg_time.unwrap_or(0.0),
        total_bytes_sent: total_bytes,
    };

    Json(ApiResponse::ok(stats))
}