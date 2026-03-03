use anyhow::{Context, Result};
use reqwest::multipart::{Form, Part};
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;

// 匹配 ImgBB 官方的 JSON 返回结构
#[derive(Deserialize, Debug)]
struct ImgBBResponse {
    data: Option<ImgBBData>,
    success: bool,
    status: u16,
    error: Option<ImgBBError>,
}

#[derive(Deserialize, Debug)]
struct ImgBBData {
    url: String, // 直接获取图片的直链
}

#[derive(Deserialize, Debug)]
struct ImgBBError {
    message: String,
}

#[derive(Clone)]
pub struct ImgBBUploader {
    pub api_key: String,
    client: Client,
}

impl ImgBBUploader {
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
        // ImgBB 要求参数名为 "key" 和 "image"
        let form = Form::new()
            .text("key", self.api_key.clone())
            .part("image", Part::bytes(file_bytes.to_vec()).file_name(file_name.to_string()));

        let res = self.client
            .post("https://api.imgbb.com/1/upload")
            .multipart(form)
            .header("User-Agent", "exloli-client/2.0")
            .send()
            .await?;

        let status = res.status();
        let text = res.text().await.context("无法读取 ImgBB 响应体")?;

        if status.is_success() {
            let parsed: ImgBBResponse = serde_json::from_str(&text)
                .context(format!("JSON 解析失败: {}", text))?;
            
            if parsed.success {
                if let Some(data) = parsed.data {
                    Ok(data.url)
                } else {
                    Err(anyhow::anyhow!("上传成功，但未返回 URL"))
                }
            } else if let Some(err) = parsed.error {
                Err(anyhow::anyhow!("ImgBB API 拒绝请求: {}", err.message))
            } else {
                Err(anyhow::anyhow!("未知的 JSON 格式: {}", text))
            }
        } else {
            Err(anyhow::anyhow!("上传失败，HTTP 状态码: {}, 内容: {}", status, text))
        }
    }
}
