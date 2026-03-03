use anyhow::{anyhow, Context, Result};
use rand::prelude::*;
use reqwest::Url;
use std::str::FromStr;
use teloxide::dispatching::DpHandlerDescription;
use teloxide::dptree::case;
use teloxide::prelude::*;
use teloxide::types::InputFile;
use teloxide::utils::command::BotCommands;
use teloxide::utils::html::escape;
use tracing::info;

use crate::bot::command::{AdminCommand, PublicCommand};
use crate::bot::handlers::{
    cmd_best_keyboard, cmd_best_text, cmd_challenge_keyboard, gallery_preview_url,
};
use crate::bot::scheduler::Scheduler;
use crate::bot::utils::{ChallengeLocker, ChallengeProvider};
use crate::bot::Bot;
use crate::config::Config;
use crate::database::{GalleryEntity, ImageEntity, MessageEntity, PollEntity};
use crate::ehentai::{EhGalleryUrl, GalleryInfo};
use crate::tags::EhTagTransDB;
use crate::uploader::ExloliUploader;
use crate::{reply_to, try_with_reply};

pub fn public_command_handler(
    _config: Config,
) -> Handler<'static, DependencyMap, Result<()>, DpHandlerDescription> {
    teloxide::filter_command::<PublicCommand, _>()
        .branch(case![PublicCommand::Query(args)].endpoint(cmd_query))
        .branch(case![PublicCommand::Ping].endpoint(cmd_ping))
        .branch(case![PublicCommand::Update(url)].endpoint(cmd_update))
        .branch(case![PublicCommand::Best(args)].endpoint(cmd_best))
        .branch(case![PublicCommand::Challenge].endpoint(cmd_challenge))
        .branch(case![PublicCommand::Upload(args)].endpoint(cmd_upload))
        .branch(case![PublicCommand::Random].endpoint(cmd_random))
        .branch(case![PublicCommand::Stats].endpoint(cmd_stats))
        .branch(case![PublicCommand::Help].endpoint(cmd_help))
}

async fn cmd_help(bot: Bot, msg: Message) -> Result<()> {
    let me = bot.get_me().await?;
    let public_help = PublicCommand::descriptions().username_from_me(&me);
    let admin_help = AdminCommand::descriptions().username_from_me(&me);
    let text = format!("<b>管理員指令：</b>\n{}\n\n<b>公共指令：</b>\n{}", admin_help, public_help);
    reply_to!(bot, msg, escape(&text)).await?;
    Ok(())
}

async fn cmd_upload(
    bot: Bot,
    msg: Message,
    uploader: ExloliUploader,
    url_text: String,
) -> Result<()> {
    if url_text.trim().is_empty() {
        reply_to!(
            bot, 
            msg, 
            "<b>使用說明：</b>\n請在指令後附上 E 站畫廊鏈接。\n\n<b>示例：</b>\n<code>/upload https://exhentai.org/g/123456/abcdef/</code>"
        ).await?;
        return Ok(());
    }

    let gallery = match EhGalleryUrl::from_str(&url_text) {
        Ok(v) => v,
        Err(_) => {
            reply_to!(bot, msg, "❌ <b>無效的鏈接</b>\n請檢查是否為正確的 E-Hentai 或 ExHentai 畫廊網址。").await?;
            return Ok(());
        }
    };

    info!("{}: /upload {}", msg.from().unwrap().id, gallery);
    
    if GalleryEntity::get(gallery.id()).await?.is_none() {
        reply_to!(bot, msg, "⚠️ <b>權限不足</b>\n非管理員只能上傳機器人數據庫中已存在的畫廊。").await?;
    } else {
        try_with_reply!(bot, msg, uploader.try_upload(&gallery, true).await);
    }
    Ok(())
}

async fn cmd_challenge(
    bot: Bot,
    msg: Message,
    trans: EhTagTransDB,
    locker: ChallengeLocker,
    scheduler: Scheduler,
    challange_provider: ChallengeProvider,
) -> Result<()> {
    info!("{}: /challenge", msg.from().unwrap().id);
    let mut challenge = challange_provider.get_challenge().await.unwrap();
    let answer = challenge[0].clone();
    challenge.shuffle(&mut thread_rng());
    
    let url = if answer.url.starts_with("http") {
        answer.url.clone()
    } else {
        format!("https://telegra.ph{}", answer.url)
    };
    
    let id = locker.add_challenge(answer.id, answer.page, answer.artist.clone());
    let keyboard = cmd_challenge_keyboard(id, &challenge, &trans);
    let reply = bot
        .send_photo(msg.chat.id, InputFile::url(url.parse()?))
        .caption("上述圖片來自下列哪位作者的本子？")
        .reply_markup(keyboard)
        .reply_to_message_id(msg.id)
        .await?;
    if !msg.chat.is_private() {
        scheduler.delete_msg(msg.chat.id, msg.id, 120);
        scheduler.delete_msg(msg.chat.id, reply.id, 120);
    }
    Ok(())
}

async fn cmd_best(
    bot: Bot,
    msg: Message,
    args: String,
    cfg: Config,
    scheduler: Scheduler,
) -> Result<()> {
    let parts: Vec<&str> = args.split_whitespace().collect();
    
    if parts.len() != 2 {
        reply_to!(
            bot, 
            msg, 
            "<b>使用說明：</b>\n查詢指定時間範圍內的熱門本子。\n\n<b>格式：</b>\n<code>/best [最近天數] [最遠天數]</code>\n\n<b>示例：</b>\n<code>/best 30 0</code> (查詢最近30天)\n<code>/best 60 30</code> (查詢上個月)"
        ).await?;
        return Ok(());
    }

    let end: u16 = match parts[0].parse() {
        Ok(v) => v,
        Err(_) => { reply_to!(bot, msg, "❌ 第一個參數必須是數字").await?; return Ok(()); }
    };
    let start: u16 = match parts[1].parse() {
        Ok(v) => v,
        Err(_) => { reply_to!(bot, msg, "❌ 第二個參數必須是數字").await?; return Ok(()); }
    };

    if start >= end {
         reply_to!(bot, msg, "❌ <b>參數錯誤</b>\n第一個數字（最近天數）必須大於第二個數字（最遠天數）。").await?;
         return Ok(());
    }

    info!("{}: /best {} {}", msg.from().unwrap().id, end, start);
    
    let text = cmd_best_text(start as i32, end as i32, 0, cfg.telegram.channel_id).await?;
    let keyboard = cmd_best_keyboard(start as i32, end as i32, 0);
    let reply =
        reply_to!(bot, msg, text).reply_markup(keyboard).disable_web_page_preview(true).await?;
        
    if !msg.chat.is_private() {
        scheduler.delete_msg(msg.chat.id, msg.id, 120);
        scheduler.delete_msg(msg.chat.id, reply.id, 120);
    }
    Ok(())
}

async fn cmd_update(bot: Bot, msg: Message, uploader: ExloliUploader, url_text: String) -> Result<()> {
    let msg_id = if url_text.trim().is_empty() {
        msg.reply_to_message()
            .and_then(|msg| msg.forward_from_message_id())
            .ok_or_else(|| anyhow!("請輸入 URL 或回覆一條畫廊消息"))
    } else {
        match Url::parse(&url_text) {
             Ok(u) => Ok(u),
             Err(_) => Err(anyhow!("無效的 URL")),
        }
        .and_then(|u| {
            u.path_segments()
                .and_then(|p| p.last())
                .and_then(|id| id.parse::<i32>().ok())
                .ok_or(anyhow!("無法從 URL 解析 ID"))
        })
    };

    let msg_id = match msg_id {
        Ok(id) => id,
        Err(_) => {
             reply_to!(bot, msg, "<b>使用說明：</b>\n請在指令後附上 URL，或回覆一條頻道轉發的畫廊消息。\n\n<b>示例：</b>\n<code>/update https://exhentai.org/g/xxxxx/xxxx/</code>").await?;
             return Ok(());
        }
    };

    info!("{}: /update (msg_id/url: {})", msg.from().unwrap().id, msg_id);

    let msg_entity = MessageEntity::get(msg_id).await?.ok_or(anyhow!("找不到對應的消息記錄"))?;
    let gl_entity =
        GalleryEntity::get(msg_entity.gallery_id).await?.ok_or(anyhow!("找不到對應的畫廊記錄"))?;

    let reply = reply_to!(bot, msg, "正在更新元數據...").await?;

    if let Err(e) = uploader.recheck(vec![gl_entity.clone()]).await {
        reply_to!(bot, msg, format!("Recheck 失敗: {}", e)).await?;
    }
    
    if let Err(e) = uploader.try_update(&gl_entity.url(), false).await {
         reply_to!(bot, msg, format!("Update 失敗: {}", e)).await?;
    }
    
    bot.edit_message_text(msg.chat.id, reply.id, "✅ 更新完成").await?;

    Ok(())
}

async fn cmd_ping(bot: Bot, msg: Message, scheduler: Scheduler) -> Result<()> {
    info!("{}: /ping", msg.from().unwrap().id);
    let reply = reply_to!(bot, msg, "🏓 <b>Pong~</b>").await?;
    if !msg.chat.is_private() {
        scheduler.delete_msg(msg.chat.id, msg.id, 120);
        scheduler.delete_msg(msg.chat.id, reply.id, 120);
    }
    Ok(())
}

async fn cmd_query(bot: Bot, msg: Message, cfg: Config, url_text: String) -> Result<()> {
    if url_text.trim().is_empty() {
        reply_to!(bot, msg, "<b>使用說明：</b>\n查詢畫廊收錄狀態。\n\n<b>示例：</b>\n<code>/query https://exhentai.org/g/12345/abcde</code>").await?;
        return Ok(());
    }

    let gallery = match EhGalleryUrl::from_str(&url_text) {
        Ok(v) => v,
        Err(_) => {
            reply_to!(bot, msg, "❌ <b>無效的 E 站鏈接</b>").await?;
            return Ok(());
        }
    };

    info!("{}: /query {}", msg.from().unwrap().id, gallery);
    
    match GalleryEntity::get(gallery.id()).await? {
        Some(gallery) => {
            let poll = PollEntity::get_by_gallery(gallery.id).await?.context("找不到投票記錄")?;
            let preview = gallery_preview_url(cfg.telegram.channel_id, gallery.id).await?;
            let url = gallery.url().url();
            reply_to!(
                bot,
                msg,
                format!(
                    "🔍 <b>查詢結果：已收錄</b>\n\n📄 <b>預覽：</b>{preview}\n🔗 <b>地址：</b>{url}\n⭐️ <b>評分：</b>{:.2} (排名: {:.2}%)",
                    poll.score * 100.,
                    poll.rank().await? * 100.
                )
            )
            .await?;
        }
        None => {
            reply_to!(bot, msg, "❌ <b>未找到</b>\n該畫廊尚未被本頻道收錄。").await?;
        }
    }
    Ok(())
}

async fn cmd_random(bot: Bot, msg: Message, cfg: Config) -> Result<()> {
    info!("{}: /random", msg.from().unwrap().id);
    
    match GalleryEntity::get_random().await? {
        Some(gallery) => {
            let poll = PollEntity::get_by_gallery(gallery.id).await?;
            let score = poll.as_ref().map(|p| p.score * 100.).unwrap_or(0.0);
            let rank = match poll {
                Some(p) => p.rank().await? * 100.,
                None => 0.0,
            };
            
            let preview = gallery_preview_url(cfg.telegram.channel_id, gallery.id).await?;
            let url = gallery.url().url();
            
            reply_to!(
                bot,
                msg,
                format!(
                    "🎲 <b>隨機抽取結果</b>\n\n<b>{}</b>\n\n消息：{}\n地址：{}\n評分：{:.2}（{:.2}%）",
                    gallery.title_jp.unwrap_or(gallery.title),
                    preview,
                    url,
                    score,
                    rank
                )
            )
            .await?;
        }
        None => {
            reply_to!(bot, msg, "資料庫是空的，先去上傳幾本吧！").await?;
        }
    }
    Ok(())
}

async fn cmd_stats(bot: Bot, msg: Message) -> Result<()> {
    info!("{}: /stats", msg.from().unwrap().id);
    
    let gallery_count = GalleryEntity::count().await?;
    let image_count = ImageEntity::count().await?;
    
    let avg_pages = if gallery_count > 0 {
        image_count as f64 / gallery_count as f64
    } else {
        0.0
    };

    let text = format!(
        "📊 <b>夏萊閱覽室數據統計</b>\n\n📚 <b>藏書總量：</b> <code>{}</code> 本\n🖼 <b>圖片總數：</b> <code>{}</code> 張\n📄 <b>平均頁數：</b> <code>{:.1}</code> 頁/本\n\n<i>Bot 正在持續運轉中...</i>",
        gallery_count,
        image_count,
        avg_pages
    );

    reply_to!(bot, msg, text).await?;
    Ok(())
}
