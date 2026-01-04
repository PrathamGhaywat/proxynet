use sqlx::{sqlite::SqlitePool, Row};
use crate::logger::RequestLog;
use std::path::Path;

pub async fn init_db() -> Result<SqlitePool, sqlx::Error> {
    let db_path = "proxynet.db";
    
    if let Some(parent) = Path::new(db_path).parent() {
        std::fs::create_dir_all(parent)?;
    }
    
    let database_url = format!("sqlite:{}", db_path);
    let pool = SqlitePool::connect(&database_url).await?;
    
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS request_logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            domain TEXT NOT NULL,
            path TEXT NOT NULL,
            method TEXT NOT NULL,
            status INTEGER NOT NULL,
            response_time_ms INTEGER NOT NULL,
            bytes_sent INTEGER NOT NULL,
            ip_address TEXT,
            user_agent TEXT,
            referer TEXT,
            timestamp INTEGER NOT NULL
        )"
    )
    .execute(&pool)
    .await?;
    
    Ok(pool)
}

pub async fn save_log(pool: &SqlitePool, log: &RequestLog) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO request_logs 
         (domain, path, method, status, response_time_ms, bytes_sent, ip_address, user_agent, referer, timestamp)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(&log.domain)
    .bind(&log.path)
    .bind(&log.method)
    .bind(log.status)
    .bind(log.response_time_ms as i64)
    .bind(log.bytes_sent as i64)
    .bind(&log.ip_address)
    .bind(&log.user_agent)
    .bind(&log.referer)
    .bind(log.timestamp.timestamp())
    .execute(pool)
    .await?;
    
    Ok(())
}

pub async fn get_recent_logs(pool: &SqlitePool, limit: i64) -> Result<Vec<RequestLog>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT domain, path, method, status, response_time_ms, bytes_sent, ip_address, user_agent, referer, timestamp
         FROM request_logs 
         ORDER BY timestamp DESC 
         LIMIT ?"
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    
    let mut logs = Vec::new();
    for row in rows {
        let timestamp_unix = row.try_get::<i64, _>("timestamp")?;
        let timestamp = chrono::DateTime::from_timestamp(timestamp_unix, 0)
            .unwrap_or_else(|| chrono::Utc::now());
        
        logs.push(RequestLog {
            domain: row.try_get("domain")?,
            path: row.try_get("path")?,
            method: row.try_get("method")?,
            status: row.try_get("status")?,
            response_time_ms: row.try_get::<i64, _>("response_time_ms")? as u128,
            bytes_sent: row.try_get::<i64, _>("bytes_sent")? as u64,
            ip_address: row.try_get("ip_address")?,
            user_agent: row.try_get("user_agent")?,
            referer: row.try_get("referer")?,
            timestamp,
        });
    }
    
    Ok(logs)
}