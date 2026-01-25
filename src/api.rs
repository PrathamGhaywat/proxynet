use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::HashMap;
use crate::database;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DomainDto {
    pub id: Option<i64>,
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
        .route("/config", get(get_all_config_endpoint).post(set_config_endpoint))
        .route("/config/{key}", get(get_config_endpoint).patch(update_config_endpoint))
        .with_state((routes, db))
}

async fn list_domains(
    State((_routes, db)): State<(Arc<RwLock<HashMap<String, String>>>, SqlitePool)>,
) -> impl IntoResponse {
    match database::get_all_domains(&db).await {
        Ok(domains) => {
            let dtos: Vec<DomainDto> = domains
                .into_iter()
                .map(|(id, domain, origin, enabled)| DomainDto {
                    id: Some(id),
                    domain,
                    origin,
                    enabled,
                })
                .collect();
            Json(ApiResponse::ok(dtos)).into_response()
        }
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::<()>::err("Failed to fetch domains".to_string())),
        )
            .into_response(),
    }
}

async fn create_domain(
    State((routes, db)): State<(Arc<RwLock<HashMap<String, String>>>, SqlitePool)>,
    Json(payload): Json<DomainDto>,
) -> impl IntoResponse {
    //save to database
    match database::create_domain(&db, &payload.domain, &payload.origin).await {
        Ok(id) => {
            //update in-memory routes
            let mut routes_lock = routes.write().await;
            routes_lock.insert(payload.domain.clone(), payload.origin.clone());

            let response = DomainDto {
                id: Some(id),
                domain: payload.domain,
                origin: payload.origin,
                enabled: true,
            };

            (StatusCode::CREATED, Json(ApiResponse::ok(response))).into_response()
        }
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::<()>::err("Failed to create domain".to_string())),
        )
            .into_response(),
    }
}

async fn get_domain(
    State((_routes, db)): State<(Arc<RwLock<HashMap<String, String>>>, SqlitePool)>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    match database::get_all_domains(&db).await {
        Ok(domains) => {
            if let Some((id, domain, origin, enabled)) = domains.iter().find(|(d_id, _, _, _)| *d_id == id) {
                let dto = DomainDto {
                    id: Some(*id),
                    domain: domain.clone(),
                    origin: origin.clone(),
                    enabled: *enabled,
                };
                (StatusCode::OK, Json(ApiResponse::ok(dto))).into_response()
            } else {
                (
                    StatusCode::NOT_FOUND,
                    Json(ApiResponse::<()>::err("Domain not found".to_string())),
                )
                    .into_response()
            }
        }
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::<()>::err("Failed to fetch domain".to_string())),
        )
            .into_response(),
    }
}

async fn update_domain(
    State((routes, db)): State<(Arc<RwLock<HashMap<String, String>>>, SqlitePool)>,
    Path(id): Path<i64>,
    Json(payload): Json<DomainDto>,
) -> impl IntoResponse {
    //update database
    match database::update_domain(&db, id, &payload.domain, &payload.origin).await {
        Ok(_) => {
            //update in-memory routes
            let mut routes_lock = routes.write().await;

            //find and remove old domain
            if let Some(domains) = database::get_all_domains(&db).await.ok() {
                if let Some((_, old_domain, _, _)) = domains.iter().find(|(d_id, _, _, _)| *d_id == id) {
                    routes_lock.remove(old_domain);
                }
            }

            //add new domain
            routes_lock.insert(payload.domain.clone(), payload.origin.clone());

            let response = DomainDto {
                id: Some(id),
                domain: payload.domain,
                origin: payload.origin,
                enabled: true,
            };

            (StatusCode::OK, Json(ApiResponse::ok(response))).into_response()
        }
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::<()>::err("Failed to update domain".to_string())),
        )
            .into_response(),
    }
}

async fn delete_domain(
    State((routes, db)): State<(Arc<RwLock<HashMap<String, String>>>, SqlitePool)>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    //delete from database
    match database::delete_domain(&db, id).await {
        Ok(_) => {
            //remove from in-memory routes
            let mut routes_lock = routes.write().await;

            //find domain to remove
            if let Some(domains) = database::get_all_domains(&db).await.ok() {
                if let Some((_, domain, _, _)) = domains.iter().find(|(d_id, _, _, _)| *d_id == id) {
                    routes_lock.remove(domain);
                }
            }

            (StatusCode::NO_CONTENT, "").into_response()
        }
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::<()>::err("Failed to delete domain".to_string())),
        )
            .into_response(),
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

async fn get_all_config_endpoint(
    State((_routes, db)): State<(Arc<RwLock<HashMap<String, String>>>, SqlitePool)>,
) -> impl IntoResponse {
    match crate::database::get_all_config(&db).await {
        Ok(config) => {
            let config_map: std::collections::HashMap<String, String> = config.into_iter().collect();
            Json(ApiResponse::ok(config_map)).into_response()
        }
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::<()>::err("Failed to fetch config".to_string())),
        )
            .into_response(),
    }
}

async fn get_config_endpoint(
    State((_routes, db)): State<(Arc<RwLock<HashMap<String, String>>>, SqlitePool)>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    match crate::database::get_config(&db, &key).await {
        Ok(Some(value)) => {
            let mut config = std::collections::HashMap::new();
            config.insert(key, value);
            Json(ApiResponse::ok(config)).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::err("Config key not found".to_string())),
        )
            .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::<()>::err("Failed to fetch config".to_string())),
        )
            .into_response(),
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ConfigUpdate {
    value: String,
}

async fn update_config_endpoint(
    State((_routes, db)): State<(Arc<RwLock<HashMap<String, String>>>, SqlitePool)>,
    Path(key): Path<String>,
    Json(payload): Json<ConfigUpdate>,
) -> impl IntoResponse {
    match crate::database::set_config(&db, &key, &payload.value).await {
        Ok(_) => {
            let mut config = std::collections::HashMap::new();
            config.insert(key, payload.value);
            Json(ApiResponse::ok(config)).into_response()
        }
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::<()>::err("Failed to update config".to_string())),
        )
            .into_response(),
    }
}

async fn set_config_endpoint(
    State((_routes, db)): State<(Arc<RwLock<HashMap<String, String>>>, SqlitePool)>,
    Json(payload): Json<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    for (key, value) in payload.iter() {
        if let Err(_) = crate::database::set_config(&db, key, value).await {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::<()>::err("Failed to update config".to_string())),
            )
                .into_response();
        }
    }

    Json(ApiResponse::ok(payload)).into_response()
}