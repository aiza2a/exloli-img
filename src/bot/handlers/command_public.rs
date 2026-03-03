use anyhow::{anyhow, Result};
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
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};
use crate::bot::utils::CallbackData;

use crate::bot::command::{AdminCommand, PublicCommand};
use crate::bot::handlers::{
    cmd_best_keyboard, cmd_best_text, cmd_challenge_keyboard, gallery_preview_url,
};
use crate::bot::scheduler::Scheduler;
use crate::bot::utils::{ChallengeLocker, ChallengeProvider};
use crate::bot::Bot;
use crate::config::Config;
use crate::database::{GalleryEntity, ImageEntity, MessageEntity, PollEntity};
use crate::ehentai::GalleryInfo; 
use crate::ehentai::EhGalleryUrl;
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
        .branch(case![PublicCommand::Random(args)].endpoint(cmd_random))
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
            "<b>使用說明：</b>\n查詢指定時間範圍內的熱門本子。\n\n<b>格式：</b>\n<code>/best [天數1] [天數2]</code>\n\n<b>示例：</b>\n<code>/best 30 0</code> (查詢最近30天)\n<code>/best 30 60</code> (查詢上個月)"
        ).await?;
        return Ok(());
    }

    let day1: i32 = match parts[0].parse() {
        Ok(v) => v,
        Err(_) => { reply_to!(bot, msg, "❌ 第一個參數必須是數字").await?; return Ok(()); }
    };
    let day2: i32 = match parts[1].parse() {
        Ok(v) => v,
        Err(_) => { reply_to!(bot, msg, "❌ 第二個參數必須是數字").await?; return Ok(()); }
    };

    info!("{}: /best {} {}", msg.from().unwrap().id, day1, day2);
    
    let text = cmd_best_text(day1, day2, 0, cfg.telegram.channel_id).await?;
    let keyboard = cmd_best_keyboard(day1, day2, 0);
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

    let msg_entity = MessageEntity::get(msg_id).await?.ok_or_else(|| anyhow!("找不到對應的消息記錄"))?;
    let gl_entity =
        GalleryEntity::get(msg_entity.gallery_id).await?.ok_or_else(|| anyhow!("找不到對應的畫廊記錄"))?;

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
            let poll = PollEntity::get_by_gallery(gallery.id).await?.ok_or_else(|| anyhow!("找不到投票記錄"))?;
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

async fn cmd_random(bot: Bot, msg: Message, cfg: Config, args: String) -> Result<()> {
    info!("{}: /random {}", msg.from().unwrap().id, args);
    
    let mut parts: Vec<&str> = args.split_whitespace().collect();
    let mut count = 1;
    
    // 檢查最後一個參數是否為數字（推送數量）
    if let Some(last) = parts.last() {
        if let Ok(c) = last.parse::<usize>() {
            count = c;
            parts.pop(); // 彈出數字，剩下的就是標籤
        }
    }
    
    // 強制限制：最小 1 篇，最大 10 篇
    count = count.clamp(1, 10);
    let tags: Vec<String> = parts.into_iter().map(|s| s.to_string()).collect();

    for i in 0..count {
        let gallery = if tags.is_empty() {
            GalleryEntity::get_random().await?
        } else {
            GalleryEntity::get_random_with_tags(&tags).await?
        };

        match gallery {
            Some(gallery) => {
                let poll = PollEntity::get_by_gallery(gallery.id).await?;
                let score = poll.as_ref().map(|p| p.score * 100.).unwrap_or(0.0);
                let rank = match &poll {
                    Some(p) => p.rank().await? * 100.,
                    None => 0.0,
                };
                
                // Telegram 会自动通过这裡的链接产生富文本预览图 (封面图)
                let preview = gallery_preview_url(cfg.telegram.channel_id.clone(), gallery.id).await?;
                let url = gallery.url().url();
                
                let text = format!(
                    "🎲 <b>隨機抽取結果</b>\n\n<b>{}</b>\n\n📄 <b>預覽：</b>{}\n🔗 <b>地址：</b>{}\n⭐️ <b>評分：</b>{:.2}（{:.2}%）",
                    gallery.title_jp.unwrap_or(gallery.title),
                    preview,
                    url,
                    score,
                    rank
                );

                let mut msg_req = bot.send_message(msg.chat.id, text);

                // 只有最後一條消息添加「再來一個本子」按鈕，避免刷屏時滿屏按鈕
                if i == count - 1 {
                    let mut tags_str = tags.join(" ");
                    if tags_str.len() > 40 {
                        tags_str = tags_str[..40].to_string(); // 防止超出 Telegram 內部按鈕資料 64 字节的限制
                    }
                    let keyboard = InlineKeyboardMarkup::new(vec![vec![
                        InlineKeyboardButton::callback("🎲 再來一個本子", CallbackData::RandomAnother(tags_str).pack()),
                    ]]);
                    msg_req = msg_req.reply_markup(keyboard);
                }

                msg_req.await?;
            }
            None => {
                let tag_str = tags.join(", ");
                if tags.is_empty() {
                    reply_to!(bot, msg, "資料庫是空的，先去上傳幾本吧！").await?;
                } else {
                    // 若標籤不存在則發送你要求的提示消息
                    reply_to!(bot, msg, format!("❌ <b>未找到匹配的本子</b>\n沒有找到包含標籤 <code>{}</code> 的畫廊，請更換關鍵詞再試一次。", tag_str)).await?;
                }
                break; // 如果沒本子了就直接中斷循環
            }
        }
        
        // 批量推送時加入微小延遲，防止觸發 Telegram 刷屏風控
        if count > 1 {
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
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
