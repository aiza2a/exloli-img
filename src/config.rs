use std::time::Duration;

use anyhow::Result;
use duration_str::deserialize_duration;
use once_cell::sync::OnceCell;
use serde::Deserialize;
use teloxide::types::{ChatId, Recipient};

pub static CHANNEL_ID: OnceCell<String> = OnceCell::new();

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub log_level: String,
    pub threads_num: usize,
    #[serde(deserialize_with = "deserialize_duration")]
    pub interval: Duration,
    pub database_url: String,
    pub exhentai: ExHentai,
    pub telegraph: Telegraph,
    pub telegram: Telegram,
    pub freeimage: FreeimageConfig, 
}

#[derive(Debug, Clone, Deserialize)]
pub struct FreeimageConfig {
    pub api_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExHentai {
    pub cookie: String,
    pub search_params: Vec<(String, String)>,
    pub search_count: usize,
    pub trans_file: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Telegraph {
    pub access_token: String,
    pub author_name: String,
    pub author_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Telegram {
    pub channel_id: Recipient,
    pub bot_id: String,
    pub token: String,
    pub group_id: ChatId,
    pub auth_group_id: ChatId,
}

impl Config {
    pub fn new(path: &str) -> Result<Self> {
        let s = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&s)?)
    }
}
