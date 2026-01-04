mod config;
mod logger;
mod database;

use axum::{
    body::Body,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Router, 
};
use hyper_util::{client::legacy::Client, rt::TokioExecutor};
use sqlx::sqlite::SqlitePool;
use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Instant};
use tokio::sync::RwLock;
use tracing::{info, warn};
use config::Config;
use logger::RequestLog;
use database::{init_db, save_log};


type HyperClient = Client<hyper_util::client::legacy::connect::HttpConnector, Body>;

#[derive(Clone)]
struct AppState {
    routes: Arc<RwLock<HashMap<String, String>>>,
    client: HyperClient,
    db: SqlitePool,
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
            .with_ip(addr.ip().to_string());

            log.log();
            let _ = save_log(&state.db, &log).await;

            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from(format!("Domain '{}' not configured", host)))
                .unwrap());
        }
    };
    drop(routes);

    //build upstream url
    let query = req.uri().query().map(|q| format!("?{}", q)).unwrap_or_default();
    let upstream_uri = format!("{}{}{}", origin, path, query);

    info!("PROXYING: {} -> {}", host, upstream_uri);

    //update req uri
    *req.uri_mut() = upstream_uri.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
    req.headers_mut().remove("host");

    //forward req
    match state.client.request(req).await {
        Ok(response) => {
            let status = response.status().as_u16();
            info!("SUCCESS: {} responded with {}", origin, status);

            //log successful request
            let mut log = RequestLog::new(
                host.to_string(),
                path,
                method,
                status,
                start_time,
            )
            .with_ip(addr.ip().to_string());

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
            .with_ip(addr.ip().to_string());

            log.log();
            let _ = save_log(&state.db, &log).await;

            Err(StatusCode::BAD_GATEWAY)
        }
    }
}