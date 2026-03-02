use anyhow::{Context, Result};
use reqwest::multipart::{Form, Part};
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;

#[derive(Deserialize, Debug)]
struct FreeimageResponse {
    status_code: u16,
    image: Option<FreeimageImage>,
    error: Option<FreeimageError>,
}

#[derive(Deserialize, Debug)]
struct FreeimageImage {
    url: String, // 直接获取图片的直链
}

#[derive(Deserialize, Debug)]
struct FreeimageError {
    message: String,
}

#[derive(Clone)]
pub struct FreeimageUploader {
    api_key: String,
    client: Client,
}

impl FreeimageUploader {
    pub fn new(api_key: &str) -> Self {
        Self {
            api_key: api_key.to_string(),
            client: Client::builder()
                .timeout(Duration::from_secs(60)) // 给予图床充裕的响应时间
                .build()
                .unwrap(),
        }
    }

    pub async fn upload_file(&self, file_name: &str, file_bytes: &[u8]) -> Result<String> {
        let form = Form::new()
            .text("key", self.api_key.clone())
            .part("source", Part::bytes(file_bytes.to_vec()).file_name(file_name.to_string()));

        let res = self.client
            .post("https://freeimage.host/api/1/upload")
            .multipart(form)
            .header("User-Agent", "exloli-client/2.0")
            .send()
            .await?;

        let status = res.status();
        let text = res.text().await.context("无法读取 Freeimage 响应体")?;

        if status.is_success() {
            // 解析 Chevereto 架构的标准 JSON
            let parsed: FreeimageResponse = serde_json::from_str(&text)
                .context(format!("JSON 解析失败: {}", text))?;
            
            if let Some(image) = parsed.image {
                Ok(image.url)
            } else if let Some(err) = parsed.error {
                Err(anyhow::anyhow!("图床 API 拒绝请求: {}", err.message))
            } else {
                Err(anyhow::anyhow!("未知的 JSON 格式: {}", text))
            }
        } else {
            Err(anyhow::anyhow!("上传失败，HTTP 状态码: {}, 内容: {}", status, text))
        }
    }
}
