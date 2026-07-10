use std::backtrace::Backtrace;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use chrono::{Datelike, Utc};
use futures::StreamExt;
use regex::Regex;
use reqwest::{Client, StatusCode};
use std::sync::Arc;
use telegraph_rs::{html_to_node, Telegraph};
use teloxide::prelude::*;
use teloxide::types::MessageId;
use teloxide::utils::html::escape;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tokio::task::JoinSet;
use tokio::time;
use tracing::{debug, error, info, warn, Instrument};

use crate::bot::Bot;
use crate::config::Config;
use crate::database::{
    GalleryEntity, ImageEntity, MessageEntity, PageEntity, PollEntity, TelegraphEntity,
};
use crate::ehentai::{EhClient, EhGallery, EhGalleryUrl, GalleryInfo};
use crate::kvault::KvaultUploader;
use crate::tags::EhTagTransDB;

#[derive(Debug, Clone)]
pub struct ExloliUploader {
    ehentai: EhClient,
    telegraph: Telegraph,
    bot: Bot,
    config: Config,
    trans: EhTagTransDB,
}

impl ExloliUploader {
    pub async fn new(
        config: Config,
        ehentai: EhClient,
        bot: Bot,
        trans: EhTagTransDB,
    ) -> Result<Self> {
        let telegraph = Telegraph::new(&config.telegraph.author_name)
            .author_url(&config.telegraph.author_url)
            .access_token(&config.telegraph.access_token)
            .create()
            .await?;
        Ok(Self { ehentai, config, telegraph, bot, trans })
    }

    /// 每隔 interval 分鐘檢查一次
    pub async fn start(&self) {
        loop {
            info!("開始掃描 E 站本子");
            self.check().await;
            info!("掃描完畢，等待 {:?} 後繼續", self.config.interval);
            time::sleep(self.config.interval).await;
        }
    }

    /// 根據設定檔，掃描前 N 個本子，並進行上傳或更新
    #[tracing::instrument(skip(self))]
    async fn check(&self) {
        let stream = self
            .ehentai
            .search_iter(&self.config.exhentai.search_params)
            .take(self.config.exhentai.search_count);
        tokio::pin!(stream);
        while let Some(next) = stream.next().await {
            // 錯誤不要上拋，避免影響後續畫廊
            if let Err(err) = self.try_update(&next, true).await {
                error!("check_and_update: {:?}\n{}", err, Backtrace::force_capture());
            }
            if let Err(err) = self.try_upload(&next, true).await {
                error!("check_and_upload: {:?}\n{}", err, Backtrace::force_capture());
            }
            time::sleep(Duration::from_secs(1)).await;
        }
    }

    /// 檢查指定畫廊是否已經上傳，如果沒有則進行上傳
    #[tracing::instrument(skip(self))]
    pub async fn try_upload(&self, gallery_url_param: &EhGalleryUrl, check: bool) -> Result<()> {
        if check {
            if let (Some(existing), Some(message)) = (
                GalleryEntity::get(gallery_url_param.id()).await?,
                MessageEntity::get_by_gallery(gallery_url_param.id()).await?,
            ) {
                let stored_pages = PageEntity::count(existing.id).await?;
                if stored_pages >= existing.pages {
                    return Ok(());
                }

                info!(
                    gallery_id = existing.id,
                    stored_pages,
                    expected_pages = existing.pages,
                    "检测到不完整画廊，进入修复流程"
                );
                return self.republish(&existing, &message).await;
            }
        }

        let gallery_data = self.ehentai.get_gallery(gallery_url_param).await?;

        // 核心修改：检测上传是否完整
        // 如果 upload_gallery_image 返回 false，说明有图片失败，直接返回，不发送消息
        // 由于没有写入数据库，下次 check 循环会再次扫描并重试
        if !self.upload_gallery_image(&gallery_data).await? {
            error!(
                "畫廊 {} 上傳不完整（存在失敗圖片），跳過本次推送，等待下次重試。",
                gallery_data.url.id()
            );
            return Ok(());
        }

        let article = self.publish_telegraph_article(&gallery_data).await?;

        // 建立 Telegram 訊息
        let text = self.create_message_text(&gallery_data, &article.url).await?;

        let msg = if let Some(parent) = &gallery_data.parent {
            if let Some(pmsg) = MessageEntity::get_by_gallery(parent.id()).await? {
                self.bot
                    .send_message(self.config.telegram.channel_id.clone(), text)
                    .reply_to_message_id(MessageId(pmsg.id))
                    .parse_mode(teloxide::types::ParseMode::Html)
                    .await?
            } else {
                self.bot
                    .send_message(self.config.telegram.channel_id.clone(), text)
                    .parse_mode(teloxide::types::ParseMode::Html)
                    .await?
            }
        } else {
            self.bot
                .send_message(self.config.telegram.channel_id.clone(), text)
                .parse_mode(teloxide::types::ParseMode::Html)
                .await?
        };

        // 資料入庫
        MessageEntity::create(msg.id.0, gallery_data.url.id()).await?;
        TelegraphEntity::create(gallery_data.url.id(), &article.url).await?;
        GalleryEntity::create(&gallery_data).await?;

        Ok(())
    }

    /// 檢查指定畫廊是否有更新（標題或標籤變動）
    #[tracing::instrument(skip(self))]
    pub async fn try_update(&self, gallery_url_param: &EhGalleryUrl, check: bool) -> Result<()> {
        let entity = match GalleryEntity::get(gallery_url_param.id()).await? {
            Some(v) => v,
            _ => return Ok(()),
        };
        let message = match MessageEntity::get_by_gallery(gallery_url_param.id()).await? {
            Some(v) => v,
            _ => return Ok(()),
        };

        // 階梯式更新頻率
        let now = Utc::now().date_naive();
        let seed = match now - message.publish_date {
            d if d < chrono::Duration::days(2) => 1,
            d if d < chrono::Duration::days(7) => 3,
            d if d < chrono::Duration::days(14) => 7,
            _ => 14,
        };
        if check && now.day() % seed != 0 {
            return Ok(());
        }

        let current_gallery_data = self.ehentai.get_gallery(gallery_url_param).await?;

        // 补充新页面时要求图片完整，避免生成包含缺页的预览。
        if !self.upload_gallery_image(&current_gallery_data).await? {
            bail!("画廊 {} 图片补全失败", current_gallery_data.url.id());
        }

        if current_gallery_data.tags != entity.tags.0 || current_gallery_data.title != entity.title
        {
            let telegraph = TelegraphEntity::get(current_gallery_data.url.id()).await?.unwrap();

            let text = self.create_message_text(&current_gallery_data, &telegraph.url).await?;

            let edit_res = self
                .bot
                .edit_message_text(
                    self.config.telegram.channel_id.clone(),
                    MessageId(message.id),
                    text,
                )
                .parse_mode(teloxide::types::ParseMode::Html)
                .await;

            if let Err(e) = edit_res {
                if e.to_string().contains("message is not modified") {
                    debug!("畫廊 {} 內容未實質改變，忽略更新報錯", current_gallery_data.url.id());
                } else {
                    return Err(e.into());
                }
            }
        }

        GalleryEntity::create(&current_gallery_data).await?;

        Ok(())
    }

    /// 重新發布指定畫廊的文章，並更新訊息
    pub async fn republish(&self, gallery: &GalleryEntity, msg: &MessageEntity) -> Result<()> {
        info!("重新發布：{}", msg.id);

        let gallery_data = self.ehentai.get_gallery(&gallery.url()).await?;

        if !self.upload_gallery_image(&gallery_data).await? {
            bail!("画廊 {} 图片补全失败", gallery_data.url.id());
        }

        let refreshed_gallery = GalleryEntity::get(gallery.id)
            .await?
            .ok_or_else(|| anyhow!("图片补全后找不到画廊 {}", gallery.id))?;
        let article = self.publish_telegraph_article(&refreshed_gallery).await?;
        let text = self.create_message_text(&refreshed_gallery, &article.url).await?;

        let edit_res = self
            .bot
            .edit_message_text(self.config.telegram.channel_id.clone(), MessageId(msg.id), text)
            .parse_mode(teloxide::types::ParseMode::Html)
            .await;

        if let Err(e) = edit_res {
            if e.to_string().contains("message is not modified") {
                debug!("重新發布時內容未改變，忽略錯誤");
            } else {
                return Err(e.into());
            }
        }

        TelegraphEntity::update(gallery.id, &article.url).await?;
        Ok(())
    }

    pub async fn check_telegraph(&self, url: &str) -> Result<bool> {
        Ok(Client::new().head(url).send().await?.status() != StatusCode::NOT_FOUND)
    }
}

// ==========================================
// 重傳與檢測模組
// ==========================================
impl ExloliUploader {
    pub async fn reupload(&self, mut galleries: Vec<GalleryEntity>) -> Result<()> {
        if galleries.is_empty() {
            galleries = GalleryEntity::list_scans().await?;
        }
        for gallery in galleries.iter().rev() {
            if let Some(score) = PollEntity::get_by_gallery(gallery.id).await? {
                if score.score > 0.8 {
                    info!("嘗試上傳畫廊：{}", gallery.url());
                    if let Err(err) = self.try_upload(&gallery.url(), true).await {
                        error!("上傳失敗：{}", err);
                    }
                    time::sleep(Duration::from_secs(60)).await;
                }
            }
        }
        Ok(())
    }

    pub async fn repair_incomplete(&self, mut galleries: Vec<GalleryEntity>) -> Result<()> {
        if galleries.is_empty() {
            galleries = GalleryEntity::list_incomplete().await?;
        }

        for gallery in galleries.iter().rev() {
            let stored_pages = PageEntity::count(gallery.id).await?;
            if stored_pages >= gallery.pages {
                continue;
            }
            let Some(message) = MessageEntity::get_by_gallery(gallery.id).await? else {
                warn!(gallery_id = gallery.id, "不完整画廊没有频道消息，跳过自动修复");
                continue;
            };

            info!(
                gallery_id = gallery.id,
                stored_pages,
                expected_pages = gallery.pages,
                "开始补全不完整画廊"
            );
            if let Err(err) = self.republish(gallery, &message).await {
                error!(gallery_id = gallery.id, error = %err, "不完整画廊补全失败");
            }
            time::sleep(Duration::from_secs(5)).await;
        }
        Ok(())
    }

    pub async fn recheck(&self, mut galleries: Vec<GalleryEntity>) -> Result<()> {
        if galleries.is_empty() {
            galleries = GalleryEntity::list_scans().await?;
        }
        for gallery in galleries.iter().rev() {
            let telegraph =
                TelegraphEntity::get(gallery.id).await?.ok_or(anyhow!("找不到 telegraph"))?;
            if let Some(msg) = MessageEntity::get_by_gallery(gallery.id).await? {
                info!("檢測畫廊：{}", gallery.url());
                if !self.check_telegraph(&telegraph.url).await? {
                    info!("重新上傳預覽：{}", gallery.url());
                    if let Err(err) = self.republish(gallery, &msg).await {
                        error!("上傳失敗：{}", err);
                    }
                    time::sleep(Duration::from_secs(60)).await;
                }
            }
            time::sleep(Duration::from_secs(1)).await;
        }
        Ok(())
    }
}

// ==========================================
// 核心併發圖床流轉與 Telegraph 渲染邏輯
// ==========================================
impl ExloliUploader {
    /// 獲取畫廊圖片，並透過 MPSC 通道流轉到 ImgBB
    /// 返回值：Result<bool>，true 表示本批次所有待处理图片均处理成功
    async fn upload_gallery_image(&self, gallery: &EhGallery) -> Result<bool> {
        let mut pages = vec![];
        for page in &gallery.pages {
            if crate::database::BadImageEntity::is_bad(page.hash()).await? == Some(2) {
                info!(page = page.page(), hash = page.hash(), "跳过已标记的广告图片");
                continue;
            }
            match ImageEntity::get_by_hash(page.hash()).await? {
                Some(img) => {
                    PageEntity::create(page.gallery_id(), page.page(), img.id).await?;
                }
                None => pages.push(page.clone()),
            }
        }

        let total_missing = pages.len();
        info!("需要下載 & 上傳 {} 張圖片", total_missing);
        if total_missing == 0 {
            return Ok(true);
        }

        // MPSC 併發通道
        let concurrent = self.config.threads_num.max(1);
        let (tx, mut rx) = tokio::sync::mpsc::channel(concurrent * 2);
        let client = self.ehentai.clone();

        // 生產者：單執行緒慢慢解析 E-Hentai 圖片源地址
        let getter = tokio::spawn(
            async move {
                for page in pages {
                    let rst = client.get_image_url(&page).await?;
                    debug!("已解析 E 站直鏈：{}", page.page());
                    tx.send((page, rst)).await?;
                }
                Result::<()>::Ok(())
            }
            .in_current_span(),
        );

        // 消費者：多 Key 輪詢 + 併發控制
        // 从配置中读取自建图床配置
        let base_url = self.config.kvault.base_url.clone();
        let api_token = self.config.kvault.api_token.clone();

        if base_url.is_empty() || api_token.is_empty() {
            bail!("未配置自建图床！請在 config.toml 中填寫 [kvault] 區塊");
        }

        let http_client = reqwest::Client::builder().timeout(Duration::from_secs(60)).build()?;

        let concurrent_limit = self.config.threads_num.max(3);

        // 实例化新的 Kvault Uploader
        let uploader_client = KvaultUploader::new(&base_url, &api_token);

        let uploader = tokio::spawn(
            async move {
                let semaphore = Arc::new(Semaphore::new(concurrent_limit));
                let mut join_set = JoinSet::new();
                let mut processed_count = 0;

                while let Some((page, (fileindex, url))) = rx.recv().await {
                    // 🚀 删除了原有的 tokio::time::sleep(Duration::from_secs(1)).await; 彻底解除限速

                    let mut suffix = url.split('.').next_back().unwrap_or("jpg");
                    if suffix == "webp" {
                        suffix = "jpg";
                    }
                    if suffix == "gif" {
                        info!(page = page.page(), "跳过 GIF 图片");
                        continue;
                    }

                    let filename = format!("{}.{}", page.hash(), suffix);

                    let permit = semaphore
                        .clone()
                        .acquire_owned()
                        .await
                        .map_err(|_| anyhow!("图片上传队列已关闭"))?;
                    let client_clone = http_client.clone();
                    let img_uploader = uploader_client.clone();

                    join_set.spawn(
                        async move {
                            let mut success = false;

                            // 依然保留 3 次重试，但对于自建图床几乎用不到
                            for attempt in 1..=3 {
                                match client_clone.get(&url).send().await {
                                    Ok(res) => match res.error_for_status() {
                                        Ok(res) => match res.bytes().await {
                                            Ok(bytes) => match img_uploader.upload_file(&filename, &bytes).await {
                                                Ok(uploaded_url) => {
                                                    let db_result: Result<()> = async {
                                                        ImageEntity::create(
                                                            fileindex,
                                                            page.hash(),
                                                            &uploaded_url,
                                                        )
                                                        .await?;
                                                        let image = ImageEntity::get_by_hash(page.hash())
                                                            .await?
                                                            .ok_or_else(|| {
                                                                anyhow!(
                                                                    "图片 {} 上传后未找到数据库记录",
                                                                    page.page()
                                                                )
                                                            })?;
                                                        PageEntity::create(
                                                            page.gallery_id(),
                                                            page.page(),
                                                            image.id,
                                                        )
                                                        .await?;
                                                        info!(
                                                            page = page.page(),
                                                            image_id = image.id,
                                                            url = %uploaded_url,
                                                            "图片上传并写入数据库成功"
                                                        );
                                                        Ok(())
                                                    }
                                                    .await;

                                                    match db_result {
                                                        Ok(()) => {
                                                            success = true;
                                                            break;
                                                        }
                                                        Err(err) => error!(
                                                            "图片 {} 数据库写入失败 (尝试 {}/3): {}",
                                                            page.page(),
                                                            attempt,
                                                            err
                                                        ),
                                                    }
                                                }
                                                Err(err) => error!(
                                                    "图床上传失败 (尝试 {}/3): {}",
                                                    attempt,
                                                    err
                                                ),
                                            },
                                            Err(err) => error!(
                                                "E站图片读取失败 (尝试 {}/3): {}",
                                                attempt,
                                                err
                                            ),
                                        },
                                        Err(err) => error!(
                                            "E站图片 HTTP 状态异常 (尝试 {}/3): {}",
                                            attempt,
                                            err
                                        ),
                                    },
                                    Err(err) => error!("E站下载失败 (尝试 {}/3): {}", attempt, err),
                                }
                                // 如果失败了才稍微等一下，避免死循环攻击
                                if attempt < 3 {
                                    tokio::time::sleep(Duration::from_secs(2)).await;
                                }
                            }

                            if !success {
                                error!("🚨 圖片 {} 彻底上传失败", page.page());
                            }

                            drop(permit);
                            success
                        }
                        .in_current_span(),
                    );
                }

                // ... 下方的 join_set.join_next().await 保持不变

                while let Some(res) = join_set.join_next().await {
                    match res {
                        Ok(true) => processed_count += 1,
                        Ok(false) => {}
                        Err(e) => error!("任務異常: {}", e),
                    }
                }

                Result::<usize>::Ok(processed_count)
            }
            .in_current_span(),
        );

        let (_, count) = tokio::try_join!(flatten(getter), flatten(uploader))?;

        if count == total_missing {
            info!("本批次處理完整 ({} / {})", count, total_missing);
            Ok(true)
        } else {
            error!("本批次處理不完整 ({} / {})", count, total_missing);
            Ok(false)
        }
    }

    /// 產生 Telegraph 文章
    async fn publish_telegraph_article<T: GalleryInfo>(
        &self,
        gallery: &T,
    ) -> Result<telegraph_rs::Page> {
        let images = ImageEntity::get_by_gallery_id(gallery.url().id()).await?;

        let mut html = String::new();
        if gallery.cover() != 0 && gallery.cover() < images.len() {
            html.push_str(&format!(r#"<img src="{}">"#, images[gallery.cover()].url()))
        }
        for img in images {
            html.push_str(&format!(r#"<img src="{}">"#, img.url()));
        }

        html.push_str(&format!("<p>ᴘᴀɢᴇꜱ : {}</p>", gallery.pages()));

        let node = html_to_node(&html);
        let title = gallery.title_jp();
        Ok(self.telegraph.create_page(&title, &node, false).await?)
    }

    /// 構建 Telegram 推送訊息
    async fn create_message_text<T: GalleryInfo>(
        &self,
        gallery: &T,
        article_url: &str,
    ) -> Result<String> {
        let re = Regex::new("[-/· ]").unwrap();
        let tags = self.trans.trans_tags(gallery.tags());
        let mut text = String::new();
        text.push_str(&format!("<b>{}</b>\n\n", escape(&gallery.title_jp())));
        for (ns, tag) in tags {
            let tag = tag
                .iter()
                .map(|s| format!("#{}", re.replace_all(s, "_")))
                .collect::<Vec<_>>()
                .join(" ");
            text.push_str(&format!("⁣⁣⁣⁣　<code>{}</code>: <i>{}</i>\n", ns, tag))
        }
        text.push_str(&format!("\n<b>〔 <a href=\"{}\">即 時 預 覽</a> 〕</b>/", article_url));
        text.push_str(&format!("<b>〔 <a href=\"{}\">來 源</a> 〕</b>", gallery.url().url()));

        Ok(text)
    }
}

async fn flatten<T>(handle: JoinHandle<Result<T>>) -> Result<T> {
    match handle.await {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(err)) => Err(err),
        Err(err) => bail!(err),
    }
}
