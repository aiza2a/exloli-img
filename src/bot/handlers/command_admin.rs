use anyhow::{Context, Result};
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
    reply_to!(bot, msg, "🔄 正在重新檢測並修復所有畫廊預覽鏈接...").await?;
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
        reply_to!(
            bot, 
            msg, 
            "<b>管理員上傳指令：</b>\n強制下載並上傳新畫廊。\n\n<b>用法：</b>\n<code>/upload [E站URL]</code>"
        ).await?;
        return Ok(());
    }

    let gallery = match EhGalleryUrl::from_str(&url_text) {
        Ok(v) => v,
        Err(_) => {
            reply_to!(bot, msg, "❌ 無效的 URL").await?;
            return Ok(());
        }
    };

    info!("{}: /upload {}", msg.from().unwrap().id, gallery);
    try_with_reply!(bot, msg, uploader.try_upload(&gallery, false).await);
    Ok(())
}

async fn cmd_delete(bot: Bot, msg: Message, command: AdminCommand) -> Result<()> {
    // 這裡我們記錄原始指令，虽然 command 参数是枚举，但通过日志可以区分上下文
    let cmd_name = match command {
        AdminCommand::Delete => "/delete (軟刪除)",
        AdminCommand::Erase => "/erase (硬刪除)",
        _ => "/unknown",
    };
    info!("{}: {}", msg.from().unwrap().id, cmd_name);

    let reply_to = msg.reply_to_message().context("請回復一條畫廊消息來執行刪除")?;

    // 嘗試獲取轉發來源
    let channel = reply_to.forward_from_chat().context("該消息不是來自頻道的轉發，無法定位畫廊")?;
    let channel_msg = reply_to.forward_from_message_id().context("無法獲取原始消息 ID")?;

    let msg_entity = MessageEntity::get(channel_msg).await?.context("數據庫中找不到該消息記錄")?;

    // 執行 Telegram 側刪除
    let _ = bot.delete_message(reply_to.chat.id, reply_to.id).await; // 刪除群組裡的轉發
    let _ = bot.delete_message(channel.id, MessageId(msg_entity.id)).await; // 刪除頻道裡的原始消息

    // 執行數據庫側刪除
    if matches!(command, AdminCommand::Delete) {
        // 軟刪除：標記為 deleted
        GalleryEntity::update_deleted(msg_entity.gallery_id, true).await?;
        reply_to!(bot, msg, "✅ <b>已軟刪除</b>\n畫廊記錄保留，標記為已刪除。").await?;
    } else {
        // 硬刪除：徹底移除
        GalleryEntity::delete(msg_entity.gallery_id).await?;
        MessageEntity::delete(channel_msg).await?;
        reply_to!(bot, msg, "🚫 <b>已硬刪除 (Erase)</b>\n畫廊記錄已徹底抹除，下次掃描將重新上傳。").await?;
    }

    Ok(())
}
