mod config;
mod logger;
mod database;
mod cache;
mod rate_limiter;

use axum::{
    body::Body,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Router, 
};
use http_body_util::BodyExt;
use hyper_util::{client::legacy::Client, rt::TokioExecutor};
use sqlx::sqlite::SqlitePool;
use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Instant};
use tokio::sync::RwLock;
use tracing::{info, warn};
use config::Config;
use logger::RequestLog;
use database::{init_db, save_log};
use cache::MemoryCache;
use rate_limiter::RateLimiter;

type HyperClient = Client<hyper_util::client::legacy::connect::HttpConnector, Body>;

#[derive(Clone)]
struct AppState {
    routes: Arc<RwLock<HashMap<String, String>>>,
    client: HyperClient,
    db: SqlitePool,
    cache: MemoryCache,
    rate_limiter: Option<RateLimiter>,
}

#[tokio::main]
async fn main() {
    //init logging
    tracing_subscriber::fmt()
        .with_target(false)
        .compact()
        .init();

    let config = Config::load("config.toml").expect("Failed to load config");
    info!("Loaded config from config.toml");

    //init database
    let db = init_db().await.expect("Failed to initialize database");
    info!("Database initialized");

    //init in-memory cache
    let cache = MemoryCache::new();
    info!("In-memory cache initialized");

    //init rate limiter if configured
    let rate_limiter = config.proxy.rate_limit_per_minute.map(|limit| {
        let rl = RateLimiter::new(limit, 60);
        rl.spawn_cleanup();
        info!("Rate limiter initialized: {} requests/minute", limit);
        rl
    });

    //create http client
    let client = Client::builder(TokioExecutor::new()).build_http();

    //build routes from config
    let mut routes = HashMap::new();
    for domain in &config.domains {
        if domain.enabled {
            routes.insert(domain.domain.clone(), domain.origin.clone());
            info!("Loaded: {} -> {}", domain.domain, domain.origin);
        } else {
            info!("Skipped (disabled): {}", domain.domain);
        }
    }

    let app_state = AppState {
        routes: Arc::new(RwLock::new(routes)),
        client,
        db,
        cache,
        rate_limiter,
    };

    //build router
    let app = Router::new()
        .fallback(proxy_handler)
        .with_state(app_state);

    //start server
    let addr = format!("{}:{}", config.proxy.host, config.proxy.port);
    info!("Proxy started on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(
        listener, 
        app.into_make_service_with_connect_info::<SocketAddr>()
    )
    .await
    .unwrap();
}

async fn proxy_handler(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    mut req: Request,
) -> Result<Response, StatusCode> {
    let start_time = Instant::now();

    //extract host from headers
    let hostname = headers
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");

    //get host without port
    let host = hostname.split(':').next().unwrap_or(hostname);

    //get user agent and referer
    let user_agent = headers
        .get("user-agent")
        .and_then(|h| h.to_str().ok())
        .map(String::from);

    let referer = headers
        .get("referer")
        .and_then(|h| h.to_str().ok())
        .map(String::from);

    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let query = req.uri().query();
    let client_ip = addr.ip().to_string();

    //enforce rate limiting if enabled
    if let Some(rl) = &state.rate_limiter {
        if !rl.allow(&client_ip).await {
            warn!("Rate limit exceeded for {}", client_ip);
            return Ok(Response::builder()
                .status(StatusCode::TOO_MANY_REQUESTS)
                .body(Body::from("Too many requests"))
                .unwrap());
        }
    }

    //look up origin for domain
    let routes = state.routes.read().await;
    let origin = match routes.get(host) {
        Some(o) => o.clone(),
        None => {
            warn!("Unknown domain: {}", host);

            //log failed request
            let log = RequestLog::new(
                host.to_string(),
                path,
                method,
                404,
                start_time,
            )
            .with_ip(client_ip.clone());

            log.log();
            let _ = save_log(&state.db, &log).await;

            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from(format!("Domain '{}' not configured", host)))
                .unwrap());
        }
    };
    drop(routes);

    //check cache for GET requests
    let cache_key = MemoryCache::generate_cache_key(host, &path, query);
    if req.method() == "GET" {
        if let Some(cached_response) = state.cache.get(&cache_key).await {
            info!("CACHE HIT: {}", cache_key);
            
            //log cached request
            let log = RequestLog::new(
                host.to_string(),
                path,
                method,
                200,
                start_time,
            )
            .with_ip(client_ip.clone());

            log.log();
            let db = state.db.clone();
            tokio::spawn(async move {
                let _ = save_log(&db, &log).await;
            });

            return Ok(Response::builder()
                .status(StatusCode::OK)
                .header("X-Cache", "HIT")
                .body(Body::from(cached_response))
                .unwrap());
        }
    }

    //build upstream url
    let query_part = query.map(|q| format!("?{}", q)).unwrap_or_default();
    let upstream_uri = format!("{}{}{}", origin, path, query_part);

    info!("PROXYING: {} -> {}", host, upstream_uri);

    //update req uri
    *req.uri_mut() = upstream_uri.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
    req.headers_mut().remove("host");

    //forward req
    let request_method = req.method().clone();
    match state.client.request(req).await {
        Ok(response) => {
            let status = response.status().as_u16();
            info!("SUCCESS: {} responded with {}", origin, status);

            //cache successful GET responses
            let should_cache = request_method == "GET" && status == 200;
            
            let (parts, body) = response.into_parts();
            
            //collect body bytes
            let body_bytes = match body.collect().await {
                Ok(collected) => collected.to_bytes(),
                Err(e) => {
                    warn!("Failed to read response body: {}", e);
                    return Err(StatusCode::BAD_GATEWAY);
                }
            };

            //cache if applicable
            if should_cache {
                if let Ok(body_str) = String::from_utf8(body_bytes.to_vec()) {
                    let cache = state.cache.clone();
                    let cache_key = cache_key.clone();
                    tokio::spawn(async move {
                        cache.set(cache_key, body_str, 300).await; //5 min TTL
                    });
                }
            }

            //log successful request
            let mut log = RequestLog::new(
                host.to_string(),
                path,
                method,
                status,
                start_time,
            )
            .with_ip(client_ip.clone());

            if let Some(ua) = user_agent {
                log = log.with_user_agent(ua);
            }

            if let Some(ref_url) = referer {
                log = log.with_referer(ref_url);
            }

            log.log();

            //save to database async
            let db = state.db.clone();
            tokio::spawn(async move {
                let _ = save_log(&db, &log).await;
            });

            //reconstruct response with cached body
            let response = Response::from_parts(parts, Body::from(body_bytes));
            Ok(response.into_response())
        }
        Err(e) => {
            warn!("ERROR: {}", e);

            //log error
            let log = RequestLog::new(
                host.to_string(),
                path,
                method,
                502,
                start_time,
            )
            .with_ip(client_ip);

            log.log();
            let _ = save_log(&state.db, &log).await;

            Err(StatusCode::BAD_GATEWAY)
        }
    }
}