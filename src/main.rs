mod config;

use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Router,
};
use hyper_util::{client::legacy::Client, rt::TokioExecutor};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use tracing::{info, warn};
use config::Config;

type HyperClient = Client<hyper_util::client::legacy::connect::HttpConnector, Body>;

#[derive(Clone)]
struct ProxyConfig {
    routes: Arc<RwLock<HashMap<String, String>>>,
    client: HyperClient,
}

#[tokio::main]
async fn main() {
    //init logging
    tracing_subscriber::fmt()
        .with_target(false)
        .compact()
        .init();

    let config = Config::load("config.toml").expect("Failed to load config");
    info!("Loaded config");

    //create http client
    let client = Client::builder(TokioExecutor::new()).build_http();

    //config routes. hardcoded for testing rn
    let mut routes = HashMap::new();
    for domain in &config.domains {
        if domain.enabled {
            routes.insert(domain.domain.clone(), domain.origin.clone());
            info!("Loaded: {} -> {}", domain.domain, domain.origin);
        } else {
            info!{"Skipped (disabled): {}", domain.domain};
        }
    }

    let proxy_config = ProxyConfig {
        routes: Arc::new(RwLock::new(routes)),
        client,
    };

    //build router
    let app = Router::new()
        .fallback(proxy_handler)
        .with_state(proxy_config);

    //start server
    let addr = format!("{}:{}", config.proxy.host, config.proxy.port);
    info!("Proxy started on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn proxy_handler(
    State(config): State<ProxyConfig>,
    headers: HeaderMap,
    mut req: Request,
) -> Result<Response, StatusCode> {
    //extract host from headers
    let hostname = headers
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");

    // get host without port
    let host = hostname.split(':').next().unwrap_or(hostname);

    // look up origin for domain
    let routes = config.routes.read().await;
    let origin = match routes.get(host) {
        Some(o) => o.clone(),
        None => {
            warn!("Unknown domain: {}", host);
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from(format!("Domain '{}' not configured", host)))
                .unwrap());
        }
    };
    drop(routes);

    //build upstream url
    let path = req.uri().path();
    let query = req.uri().query().map(|q| format!("?{}", q)).unwrap_or_default();
    let upstream_uri = format!("{}{}{}", origin, path, query);

    info!("PROXYING: {} → {}", host, upstream_uri);

    //update req uri
    *req.uri_mut() = upstream_uri.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
    req.headers_mut().remove("host");

    //forward req
    match config.client.request(req).await {
        Ok(response) => {
            info!("SUCCESS: {} responded with {}", origin, response.status());
            Ok(response.into_response())
        }
        Err(e) => {
            warn!("ERROR: {}", e);
            Err(StatusCode::BAD_GATEWAY)
        }
    }
}