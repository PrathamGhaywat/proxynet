use serde::Deserialize;
use std::fs;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub proxy: ProxySettings,
    pub domains: Vec<DomainConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ProxySettings {
    pub host: String,
    pub port: u16,
    pub rate_limit_per_minute: Option<u32>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DomainConfig {
    pub domain: String,
    pub origin: String,
    pub enabled: bool,
}

impl Config {
    pub fn load(path: &str) -> Result<Self, Box<dyn std::error::Error>>{
        let contents = fs::read_to_string(path)?;
        let config: Config = toml::from_str(&contents)?;
        Ok(config)
    }
}