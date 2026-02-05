mod logger;
mod database;
mod cache;
mod rate_limiter;
mod api;

use axum::{
    body::Body,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, StatusCode},
    response::{Response},
    Router, 
};
use http_body_util::BodyExt;
use hyper_util::{client::legacy::Client, rt::TokioExecutor};
use sqlx::sqlite::SqlitePool;
use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Instant};
use tokio::sync::RwLock;
use tracing::{info, warn};
use logger::RequestLog;
use database::{init_db, save_log};
use cache::MemoryCache;
use rate_limiter::RateLimiter;
use api::api_router;

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

    //init database first
    let db = init_db().await.expect("Failed to initialize database");
    info!("Database initialized");

    //load config from database
    let host = database::get_config(&db, "host")
        .await
        .ok()
        .flatten()
        .unwrap_or("0.0.0.0".to_string());

    let port = database::get_config(&db, "port")
        .await
        .ok()
        .flatten()
        .unwrap_or("8080".to_string())
        .parse::<u16>()
        .unwrap_or(8080);

    let api_port = database::get_config(&db, "api_port")
        .await
        .ok()
        .flatten()
        .unwrap_or("8081".to_string())
        .parse::<u16>()
        .unwrap_or(8081);

    let rate_limit = database::get_config(&db, "rate_limit_per_minute")
        .await
        .ok()
        .flatten()
        .and_then(|s| s.parse::<u32>().ok());

    info!("Loaded config from database: {}:{}, api_port: {}, rate_limit: {:?}", host, port, api_port, rate_limit);

    //init in-memory cache
    let cache = MemoryCache::new();
    info!("In-memory cache initialized");

    //init rate limiter
    let rate_limiter = rate_limit.map(|limit| {
        let rl = RateLimiter::new(limit, 60);
        rl.spawn_cleanup();
        info!("Rate limiter initialized: {} requests/minute", limit);
        rl
    });

    //create http client
    let client = Client::builder(TokioExecutor::new()).build_http();

    //build routes from database
    let mut routes = HashMap::new();
    match database::load_domains(&db).await {
        Ok(domains) => {
            for (domain, origin) in domains {
                routes.insert(domain.clone(), origin.clone());
                info!("Loaded from DB: {} -> {}", domain, origin);
            }
        }
        Err(e) => {
            warn!("Failed to load domains from DB: {}", e);
        }
    }

    let app_state = AppState {
        routes: Arc::new(RwLock::new(routes)),
        client,
        db: db.clone(),
        cache,
        rate_limiter,
    };

    //build proxy router
    let proxy_app = Router::new()
        .fallback(proxy_handler)
        .with_state(app_state.clone());

    //build API router
    let api_app = api_router(app_state.routes.clone(), db.clone());

    //start proxy server
    let proxy_addr = format!("{}:{}", host, port);
    info!("Proxy server started on http://{}", proxy_addr);

    let proxy_listener = tokio::net::TcpListener::bind(&proxy_addr).await.unwrap();
    let proxy_server = axum::serve(
        proxy_listener, 
        proxy_app.into_make_service_with_connect_info::<SocketAddr>()
    );

    //start API server
    let api_addr = format!("{}:{}", host, api_port);
    info!("API server started on http://{}", api_addr);

    let api_listener = tokio::net::TcpListener::bind(&api_addr).await.unwrap();
    let api_server = axum::serve(
        api_listener,
        api_app.into_make_service()
    );

    //run both servers concurrently
    tokio::select! {
        result = proxy_server => {
            if let Err(e) = result {
                warn!("Proxy server error: {}", e);
            }
        }
        result = api_server => {
            if let Err(e) = result {
                warn!("API server error: {}", e);
            }
        }
    }
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
            .with_ip(client_ip)
            .with_bytes(0);

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
            
            let bytes = cached_response.len() as u64;

            //log cached request
            let log = RequestLog::new(
                host.to_string(),
                path,
                method,
                200,
                start_time,
            )
            .with_ip(client_ip)
            .with_bytes(bytes);

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
    match state.client.request(req).await {
        Ok(response) => {
            let status = response.status().as_u16();
            info!("SUCCESS: {} responded with {}", origin, status);

            let collected = response.into_body().collect().await.map_err(|_| StatusCode::BAD_GATEWAY)?;
            let bytes = collected.to_bytes();
            let bytes_len = bytes.len() as u64;

            //cache successful GET responses
            if method == "GET" && status == 200 {
                if let Ok(body_str) = String::from_utf8(bytes.to_vec()) {
                    let cache = state.cache.clone();
                    let cache_key = cache_key.clone();
                    tokio::spawn(async move {
                        cache.set(cache_key, body_str, 300).await;
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
            .with_ip(client_ip)
            .with_bytes(bytes_len);

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

            Ok(Response::new(Body::from(bytes)))
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
            .with_ip(client_ip)
            .with_bytes(0);

            log.log();
            let _ = save_log(&state.db, &log).await;

            Err(StatusCode::BAD_GATEWAY)
        }
    }
}