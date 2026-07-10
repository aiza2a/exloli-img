use anyhow::{Context, Result};
use teloxide::prelude::*;
use tracing::info;

use crate::bot::handlers::utils;
use crate::bot::Bot;
use crate::database::{FavoriteEntity, GalleryEntity, PollEntity};
use crate::reply_to;

pub async fn custom_pool_sender(bot: Bot, message: Message) -> Result<()> {
    info!("频道消息更新，发送投票");

    let msg_id = message.forward_from_message_id().context("找不到消息")?;
    let gallery = GalleryEntity::get_by_msg(msg_id).await?.context("找不到画廊")?;

    let poll_id = match PollEntity::get_by_gallery(gallery.id).await? {
        Some(v) => v.id,
        None => match gallery.parent {
            Some(id) => match PollEntity::get_by_gallery(id).await? {
                Some(v) => v.id,
                None => gallery.id as i64,
            },
            None => gallery.id as i64,
        },
    };

    PollEntity::create(poll_id, gallery.id).await?;

    let votes = PollEntity::get_vote(poll_id).await?;
    // 🌟 新增查詢人數並傳給 poll_keyboard
    let fav_count = FavoriteEntity::count_by_gallery(gallery.id).await.unwrap_or(0);
    // 🌟 修改點：在這裡傳入 gallery.id
    let markup = utils::poll_keyboard(poll_id, &votes, gallery.id, fav_count);

    let score = PollEntity::update_score(poll_id).await? * 100.;
    let sum = votes.iter().sum::<i32>();
    reply_to!(bot, message, format!("当前 {sum} 人投票，{score:.2} 分"))
        .reply_markup(markup)
        .await?;

    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        bot.unpin_chat_message(message.chat.id).message_id(message.id).await?;
        Result::<()>::Ok(())
    })
    .await??;

    Ok(())
}
