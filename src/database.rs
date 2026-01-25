use sqlx::{sqlite::SqlitePool, Row};
use crate::logger::RequestLog;
use chrono::Utc;

pub async fn init_db() -> Result<SqlitePool, sqlx::Error> {
    let database_url = "sqlite:proxynet.db";
    let pool = SqlitePool::connect(database_url).await?;
    
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
    
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS domains (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            domain TEXT UNIQUE NOT NULL,
            origin TEXT NOT NULL,
            enabled BOOLEAN NOT NULL DEFAULT 1,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        )"
    )
    .execute(&pool)
    .await?;
    
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS config (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        )"
    )
    .execute(&pool)
    .await?;

    sqlx::query("INSERT OR IGNORE INTO config (key, value, updated_at) VALUES (?, ?, ?)")
        .bind("host")
        .bind("0.0.0.0")
        .bind(chrono::Utc::now().timestamp())
        .execute(&pool)
        .await?;

    sqlx::query("INSERT OR IGNORE INTO config (key, value, updated_at) VALUES (?, ?, ?)")
        .bind("port")
        .bind("8080")
        .bind(chrono::Utc::now().timestamp())
        .execute(&pool)
        .await?;

    sqlx::query("INSERT OR IGNORE INTO config (key, value, updated_at) VALUES (?, ?, ?)")
        .bind("rate_limit_per_minute")
        .bind("10")
        .bind(chrono::Utc::now().timestamp())
        .execute(&pool)
        .await?;

    Ok(pool)
}

pub async fn load_domains(pool: &SqlitePool) -> Result<Vec<(String, String)>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT domain, origin FROM domains WHERE enabled = 1"
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let domain: String = row.get("domain");
            let origin: String = row.get("origin");
            (domain, origin)
        })
        .collect())
}

pub async fn create_domain(
    pool: &SqlitePool,
    domain: &str,
    origin: &str,
) -> Result<i64, sqlx::Error> {
    let now = Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO domains (domain, origin, enabled, created_at, updated_at) VALUES (?, ?, 1, ?, ?)"
    )
    .bind(domain)
    .bind(origin)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;

    Ok(sqlx::query_scalar::<_, i64>("SELECT last_insert_rowid()")
        .fetch_one(pool)
        .await?)
}

pub async fn update_domain(
    pool: &SqlitePool,
    id: i64,
    domain: &str,
    origin: &str,
) -> Result<(), sqlx::Error> {
    let now = Utc::now().timestamp();
    sqlx::query(
        "UPDATE domains SET domain = ?, origin = ?, updated_at = ? WHERE id = ?"
    )
    .bind(domain)
    .bind(origin)
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn delete_domain(pool: &SqlitePool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM domains WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;

    Ok(())
}

pub async fn get_all_domains(pool: &SqlitePool) -> Result<Vec<(i64, String, String, bool)>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, domain, origin, enabled FROM domains"
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let id: i64 = row.get("id");
            let domain: String = row.get("domain");
            let origin: String = row.get("origin");
            let enabled: bool = row.get("enabled");
            (id, domain, origin, enabled)
        })
        .collect())
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

pub async fn get_config(pool: &SqlitePool, key: &str) -> Result<Option<String>, sqlx::Error> {
    let value: Option<String> = sqlx::query_scalar("SELECT value FROM config WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(value)
}

pub async fn set_config(pool: &SqlitePool, key: &str, value: &str) -> Result<(), sqlx::Error> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query("INSERT OR REPLACE INTO config (key, value, updated_at) VALUES (?, ?, ?)")
        .bind(key)
        .bind(value)
        .bind(now)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_all_config(pool: &SqlitePool) -> Result<Vec<(String, String)>, sqlx::Error> {
    let rows = sqlx::query("SELECT key, value FROM config")
        .fetch_all(pool)
        .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let key: String = row.get("key");
            let value: String = row.get("value");
            (key, value)
        })
        .collect())
}