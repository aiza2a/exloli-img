use anyhow::Result;
use std::str::FromStr;
use teloxide::dispatching::DpHandlerDescription;
use teloxide::dptree::case;
use teloxide::prelude::*;
use teloxide::types::MessageId;
use tracing::info;

use crate::bot::command::AdminCommand;
use crate::bot::filter::filter_admin_msg;
use crate::bot::Bot;
use crate::database::{GalleryEntity, MessageEntity};
use crate::ehentai::EhGalleryUrl;
use crate::uploader::ExloliUploader;
use crate::{reply_to, try_with_reply};

const INDENT: &str = "\u{2063}\u{3000}";

pub fn admin_command_handler() -> Handler<'static, DependencyMap, Result<()>, DpHandlerDescription>
{
    teloxide::filter_command::<AdminCommand, _>()
        .chain(filter_admin_msg())
        .branch(case![AdminCommand::Upload(url)].endpoint(cmd_upload))
        .branch(case![AdminCommand::Delete].endpoint(cmd_delete))
        .branch(case![AdminCommand::Erase].endpoint(cmd_delete))
        .branch(case![AdminCommand::ReCheck].endpoint(cmd_recheck))
}

async fn cmd_recheck(bot: Bot, msg: Message, uploader: ExloliUploader) -> Result<()> {
    info!("{}: /recheck", msg.from().unwrap().id);
    reply_to!(bot, msg, format!("<b>🔄 正在重新檢測</b>\n{i}正在修復所有失效的預覽鏈接...", i = INDENT)).await?;
    try_with_reply!(bot, msg, uploader.recheck(vec![]).await);
    Ok(())
}

async fn cmd_upload(
    bot: Bot,
    msg: Message,
    uploader: ExloliUploader,
    url_text: String,
) -> Result<()> {
    if url_text.trim().is_empty() {
        reply_to!(bot, msg, format!("<b>👮‍♂️ 管理員上傳提示</b>\n{i}用法：<code>/upload [E站URL]</code>", i = INDENT)).await?;
        return Ok(());
    }

    let gallery = match EhGalleryUrl::from_str(&url_text) {
        Ok(v) => v,
        Err(_) => {
            reply_to!(bot, msg, format!("<b>🚫 錯誤</b>\n{i}無效的畫廊鏈接。", i = INDENT)).await?;
            return Ok(());
        }
    };

    info!("{}: /upload {}", msg.from().unwrap().id, gallery);
    try_with_reply!(bot, msg, uploader.try_upload(&gallery, false).await);
    Ok(())
}

async fn cmd_delete(bot: Bot, msg: Message, command: AdminCommand) -> Result<()> {
    let cmd_name = if matches!(command, AdminCommand::Delete) { "/delete" } else { "/erase" };
    info!("{}: {}", msg.from().unwrap().id, cmd_name);

    // 🔥 修正：不再直接用 ? 報錯，而是給用戶發送消息引導，避免終端日誌 ERROR
    let reply_to = match msg.reply_to_message() {
        Some(r) => r,
        None => {
            reply_to!(bot, msg, format!("<b>⚠️ 操作無效</b>\n{i}請「回覆」一條由本頻道轉發的消息來執行刪除。", i = INDENT)).await?;
            return Ok(());
        }
    };

    let channel = match reply_to.forward_from_chat() {
        Some(c) => c,
        None => {
            reply_to!(bot, msg, format!("<b>⚠️ 操作無效</b>\n{i}該消息不是頻道轉發，無法定位畫廊。", i = INDENT)).await?;
            return Ok(());
        }
    };

    let channel_msg = match reply_to.forward_from_message_id() {
        Some(id) => id,
        None => {
            reply_to!(bot, msg, format!("<b>⚠️ 錯誤</b>\n{i}無法讀取轉發的消息 ID。", i = INDENT)).await?;
            return Ok(());
        }
    };

    let msg_entity = match MessageEntity::get(channel_msg).await? {
        Some(m) => m,
        None => {
            reply_to!(bot, msg, format!("<b>⚠️ 數據缺失</b>\n{i}數據庫中找不到該消息的記錄。", i = INDENT)).await?;
            return Ok(());
        }
    };

    // 執行刪除
    let _ = bot.delete_message(reply_to.chat.id, reply_to.id).await; 
    let _ = bot.delete_message(channel.id, MessageId(msg_entity.id)).await; 

    if matches!(command, AdminCommand::Delete) {
        GalleryEntity::update_deleted(msg_entity.gallery_id, true).await?;
        reply_to!(bot, msg, format!("<b>✅ 已執行軟刪除</b>\n{i}畫廊 ID: <code>{}</code> 已標記刪除。", msg_entity.gallery_id, i = INDENT)).await?;
    } else {
        GalleryEntity::delete(msg_entity.gallery_id).await?;
        MessageEntity::delete(channel_msg).await?;
        reply_to!(bot, msg, format!("<b>💥 已執行硬刪除 (Erase)</b>\n{i}記錄已從庫中徹底抹除。", i = INDENT)).await?;
    }

    Ok(())
}
