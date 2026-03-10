use anyhow::{Context, Result};
use reqwest::multipart::{Form, Part};
use reqwest::Client;
use std::time::Duration;
use serde_json::Value;

#[derive(Clone)]
pub struct KvaultUploader {
    pub base_url: String,
    pub api_token: String,
    client: Client,
}

impl KvaultUploader {
    pub fn new(base_url: &str, api_token: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_token: api_token.to_string(),
            client: Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .unwrap(),
        }
    }

    pub async fn upload_file(&self, file_name: &str, file_bytes: &[u8]) -> Result<String> {
        let form = Form::new()
            .part("file", Part::bytes(file_bytes.to_vec()).file_name(file_name.to_string()));

        let upload_url = format!("{}/api/v1/upload", self.base_url);

        let res = self.client
            .post(&upload_url)
            .header("Authorization", format!("Bearer {}", self.api_token))
            .multipart(form)
            .header("User-Agent", "exloli-client/3.0")
            .send()
            .await?;

        let status = res.status();
        let text = res.text().await.context("无法读取图床响应体")?;

        if status.is_success() {
            let parsed: Value = serde_json::from_str(&text)
                .context(format!("JSON 解析失败: {}", text))?;
            
            // ✨ 核心修改：新增对 K-Vault 最新 API 结构 (links/download) 的解析提取
            let extracted_url = if let Some(url) = parsed.pointer("/links/download").and_then(|v| v.as_str()) {
                Some(url) // 优先匹配 K-Vault 标准格式的直链
            } else if let Some(url) = parsed.pointer("/links/share").and_then(|v| v.as_str()) {
                Some(url) // 备用分享链接
            } else if let Some(url) = parsed.pointer("/files/0/src").and_then(|v| v.as_str()) {
                Some(url) 
            } else if let Some(url) = parsed.pointer("/0/src").and_then(|v| v.as_str()) {
                Some(url) 
            } else if let Some(src) = parsed.get("src").and_then(|v| v.as_str()) {
                Some(src) 
            } else if let Some(url) = parsed.pointer("/data/url").and_then(|v| v.as_str()) {
                Some(url) 
            } else {
                None
            };

            if let Some(url_str) = extracted_url {
                let mut full_url = url_str.to_string();
                
                // 如果返回的是相对路径，则补全域名；
                // 你目前的接口返回的是绝对路径 (https://...)，所以这段代码不会触发，完美兼容。
                if full_url.starts_with('/') {
                    full_url = format!("{}{}", self.base_url, full_url);
                }
                
                Ok(full_url)
            } else {
                Err(anyhow::anyhow!("未能在 JSON 中找到图片链接，图床返回了: {}", text))
            }
        } else {
            Err(anyhow::anyhow!("上传失败，HTTP 状态码: {}, 错误信息: {}", status, text))
        }
    }
}
