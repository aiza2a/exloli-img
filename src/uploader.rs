use std::backtrace::Backtrace;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use chrono::{Datelike, Utc};
use futures::StreamExt;
use regex::Regex;
use reqwest::{Client, StatusCode};
use telegraph_rs::{html_to_node, Telegraph};
use teloxide::prelude::*;
use teloxide::types::MessageId;
use tokio::task::JoinHandle;
use tokio::time;
use tracing::{debug, error, info, Instrument};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::bot::Bot;
use crate::config::Config;
use crate::database::{
    GalleryEntity, ImageEntity, MessageEntity, PageEntity, PollEntity, TelegraphEntity,
};
use crate::ehentai::{EhClient, EhGallery, EhGalleryUrl, GalleryInfo};
use crate::imgbb::ImgBBUploader;
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
        Ok(Self {
            ehentai,
            config,
            telegraph,
            bot,
            trans,
        })
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
                error!(
                    "check_and_update: {:?}\n{}",
                    err,
                    Backtrace::force_capture()
                );
            }
            if let Err(err) = self.try_upload(&next, true).await {
                error!(
                    "check_and_upload: {:?}\n{}",
                    err,
                    Backtrace::force_capture()
                );
            }
            time::sleep(Duration::from_secs(1)).await;
        }
    }

    /// 檢查指定畫廊是否已經上傳，如果沒有則進行上傳
    #[tracing::instrument(skip(self))]
    pub async fn try_upload(&self, gallery_url_param: &EhGalleryUrl, check: bool) -> Result<()> {
        if check
            && GalleryEntity::check(gallery_url_param.id()).await?
            && MessageEntity::get_by_gallery(gallery_url_param.id())
                .await?
                .is_some()
        {
            return Ok(());
        }

        let gallery_data = self.ehentai.get_gallery(gallery_url_param).await?;
        
        // 核心修改：检测上传是否完整
        // 如果 upload_gallery_image 返回 false，说明有图片失败，直接返回，不发送消息
        // 由于没有写入数据库，下次 check 循环会再次扫描并重试
        if !self.upload_gallery_image(&gallery_data).await? {
            error!("畫廊 {} 上傳不完整（存在失敗圖片），跳過本次推送，等待下次重試。", gallery_data.url.id());
            return Ok(());
        }

        let article = self.publish_telegraph_article(&gallery_data).await?;
        
        // 建立 Telegram 訊息
        let text = self
            .create_message_text(&gallery_data, &article.url)
            .await?;

        let msg = if let Some(parent) = &gallery_data.parent {
            if let Some(pmsg) = MessageEntity::get_by_gallery(parent.id()).await? {
                self.bot
                    .send_message(self.config.telegram.channel_id.clone(), text)
                    .reply_to_message_id(MessageId(pmsg.id))
                    .await?
            } else {
                self.bot
                    .send_message(self.config.telegram.channel_id.clone(), text)
                    .await?
            }
        } else {
            self.bot
                .send_message(self.config.telegram.channel_id.clone(), text)
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
        
        // 上傳新發現的圖片 (忽略返回值，更新流程尽量执行)
        let _ = self.upload_gallery_image(&current_gallery_data).await?;

        if current_gallery_data.tags != entity.tags.0 || current_gallery_data.title != entity.title
        {
            let telegraph = TelegraphEntity::get(current_gallery_data.url.id())
                .await?
                .unwrap();
                
            let text = self
                .create_message_text(&current_gallery_data, &telegraph.url)
                .await?;
                
            self.bot
                .edit_message_text(
                    self.config.telegram.channel_id.clone(),
                    MessageId(message.id),
                    text,
                )
                .await?;
        }

        GalleryEntity::create(&current_gallery_data).await?;

        Ok(())
    }

    /// 重新發布指定畫廊的文章，並更新訊息
    pub async fn republish(&self, gallery: &GalleryEntity, msg: &MessageEntity) -> Result<()> {
        info!("重新發布：{}", msg.id);
        
        let eh_gallery_url = gallery.url();
        let gallery_data_for_catbox = self.ehentai.get_gallery(&eh_gallery_url).await?;
        
        // 补全缺失图片
        let _ = self.upload_gallery_image(&gallery_data_for_catbox).await?;

        let article = self.publish_telegraph_article(gallery).await?;
        let text = self
            .create_message_text(gallery, &article.url)
            .await?;
            
        self.bot
            .edit_message_text(
                self.config.telegram.channel_id.clone(),
                MessageId(msg.id),
                text,
            )
            .await?;
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

    pub async fn recheck(&self, mut galleries: Vec<GalleryEntity>) -> Result<()> {
        if galleries.is_empty() {
            galleries = GalleryEntity::list_scans().await?;
        }
        for gallery in galleries.iter().rev() {
            let telegraph = TelegraphEntity::get(gallery.id)
                .await?
                .ok_or(anyhow!("找不到 telegraph"))?;
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
    /// 返回值：Result<bool>，true 表示本批次所有待处理图片均处理成功（上传成功或GIF被正常跳过）
    async fn upload_gallery_image(&self, gallery: &EhGallery) -> Result<bool> {
        let mut pages = vec![];
        for page in &gallery.pages {
            match ImageEntity::get_by_hash(page.hash()).await? {
                Some(img) => {
                    PageEntity::create(page.gallery_id(), page.page(), img.id).await?;
                }
                None => pages.push(page.clone()),
            }
        }
        
        let total_missing = pages.len();
        info!("需要下載 & 上傳 {} 張圖片", total_missing);
        if total_missing == 0 { return Ok(true); }

        // MPSC 併發通道
        let concurrent = self.config.threads_num;
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

        // 消費者：利用 Semaphore (信號量) 控制並發數的高效上傳器
        let api_key = self.config.imgbb.api_key.clone();
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;
            
        let concurrent_limit = self.config.threads_num.max(1); 
        
        let uploader = tokio::spawn(
            async move {
                let imgbb = ImgBBUploader::new(&api_key);
                let semaphore = Arc::new(Semaphore::new(concurrent_limit));
                let mut join_set = JoinSet::new();
                let mut processed_count = 0; // 记录成功处理（上传或跳过）的数量

                while let Some((page, (fileindex, url))) = rx.recv().await {
                    let mut suffix = url.split('.').last().unwrap_or("jpg");
                    
                    if suffix == "webp" { suffix = "jpg"; }
                    if suffix == "gif" { 
                        // GIF 被判定为“已处理”（虽然是跳过），计数+1
                        processed_count += 1;
                        continue; 
                    } 
                    
                    let filename = format!("{}.{}", page.hash(), suffix);
                    
                    let permit = semaphore.clone().acquire_owned().await.unwrap();
                    let client_clone = http_client.clone();
                    let imgbb_clone = imgbb.clone();

                    join_set.spawn(async move {
                        let mut success = false;
                        for attempt in 1..=3 {
                            match client_clone.get(&url).send().await {
                                Ok(res) => {
                                    if let Ok(bytes) = res.bytes().await {
                                        match imgbb_clone.upload_file(&filename, &bytes).await {
                                            Ok(uploaded_url) => {
                                                info!("已上傳至 ImgBB: {} -> {} (第 {} 次嘗試成功)", page.page(), uploaded_url, attempt);
                                                let _ = ImageEntity::create(fileindex, page.hash(), &uploaded_url).await;
                                                let _ = PageEntity::create(page.gallery_id(), page.page(), fileindex).await;
                                                success = true;
                                                break; 
                                            }
                                            Err(err) => error!("圖片 {} 上傳 ImgBB 失敗 (嘗試 {}/3): {}", page.page(), attempt, err),
                                        }
                                    }
                                }
                                Err(err) => error!("圖片 {} 從 E 站下載失敗 (嘗試 {}/3): {}", page.page(), attempt, err),
                            }
                            if attempt < 3 { tokio::time::sleep(Duration::from_secs(3)).await; }
                        }
                        
                        if !success { error!("🚨 圖片 {} 徹底上傳失敗，已放棄！", page.page()); }
                        
                        drop(permit);
                        success // 返回本次任务是否成功
                    }.in_current_span());
                }
                
                // 等待所有并发任务完成，并统计成功数
                while let Some(res) = join_set.join_next().await {
                    match res {
                        Ok(true) => processed_count += 1, // 任务成功，计数+1
                        Ok(false) => {}, // 任务失败，不计数
                        Err(e) => error!("並發任務執行異常: {}", e),
                    }
                }
                
                // 返回总处理成功数
                Result::<usize>::Ok(processed_count)
            }
            .in_current_span(),
        );

        let (_, count) = tokio::try_join!(flatten(getter), flatten(uploader))?;
        
        // 最终比对：成功处理数 是否等于 本次待处理总数
        if count == total_missing {
            info!("本批次圖片處理完整 ({} / {})", count, total_missing);
            Ok(true)
        } else {
            error!("本批次圖片處理不完整 ({} / {})，存在失敗項目", count, total_missing);
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
        text.push_str(&format!("<b>{}</b>\n\n", gallery.title_jp()));
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
