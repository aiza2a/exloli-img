use anyhow::{Context, Result};
use reqwest::multipart::{Form, Part};
use reqwest::Client;
use std::time::Duration;
// 引入 serde_json::Value 来处理动态解析
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
            // 确保去除结尾的斜杠，方便后续拼接
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

        // K-Vault 的标准 v1 接口
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
            
            // 动态匹配 K-Vault 及衍生产物可能的返回结构
            let extracted_url = if let Some(url) = parsed.pointer("/files/0/src").and_then(|v| v.as_str()) {
                Some(url) // 格式: { "files": [ { "src": "/file/..." } ] }
            } else if let Some(url) = parsed.pointer("/0/src").and_then(|v| v.as_str()) {
                Some(url) // 格式: [ { "src": "/file/..." } ]
            } else if let Some(src) = parsed.get("src").and_then(|v| v.as_str()) {
                Some(src) // 格式: { "src": "/file/..." }
            } else if let Some(url) = parsed.pointer("/data/url").and_then(|v| v.as_str()) {
                Some(url) // 备用兼容
            } else {
                None
            };

            if let Some(url_str) = extracted_url {
                let mut full_url = url_str.to_string();
                
                // ✨ 核心修改：如果是相对路径，补全你的 Cloudflare 域名
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
