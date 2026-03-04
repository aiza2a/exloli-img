use anyhow::{Context, Result};
use teloxide::dispatching::DpHandlerDescription;
use teloxide::dptree::case;
use teloxide::prelude::*;
use teloxide::utils::html::{link, user_mention};
use tracing::info;

use super::utils::gallery_preview_url;
use crate::bot::handlers::{cmd_best_keyboard, cmd_best_text, poll_keyboard};
use crate::bot::utils::{CallbackData, ChallengeLocker, RateLimiter};
use crate::bot::Bot;
use crate::config::Config;
use crate::database::{ChallengeHistory, GalleryEntity, PollEntity, VoteEntity, ImageEntity};
use crate::ehentai::GalleryInfo;
use crate::tags::EhTagTransDB;

pub fn callback_query_handler() -> Handler<'static, DependencyMap, Result<()>, DpHandlerDescription>
{
    dptree::entry()
        .branch(case![CallbackData::VoteForPoll(poll, option)].endpoint(callback_vote_for_poll))
        .branch(case![CallbackData::Challenge(id, artist)].endpoint(callback_challenge))
        .branch(case![CallbackData::RandomAnother(tags)].endpoint(callback_random_another)) 
        .endpoint(callback_change_page)
}

async fn callback_challenge(
    bot: Bot,
    query: CallbackQuery,
    trans: EhTagTransDB,
    locker: ChallengeLocker,
    cfg: Config,
    (id, artist): (i64, String),
) -> Result<()> {
    let message = query.message.context("消息过旧")?;
    info!("{}: <- challenge {} {}", query.from.id, id, artist);

    if let Some((gallery, page, answer)) = locker.get_challenge(id) {
        let success = answer == artist;
        let gallery_entity = GalleryEntity::get(gallery).await?.context("找不到画廊")?;
        let preview = gallery_preview_url(cfg.telegram.channel_id, gallery).await?;
        let poll = PollEntity::get_by_gallery(gallery).await?.context("找不到投票")?;
        ChallengeHistory::create(query.from.id.0 as i64, gallery, page, success, message.chat.id.0)
            .await?;

        let (stat_success, stat_total) =
            ChallengeHistory::answer_stats(query.from.id.0 as i64, message.chat.id.0).await?;

        let mention = user_mention(query.from.id.0 as i64, &query.from.full_name());
        let result = if success { "答对了！" } else { "答错了……" };
        let artist = trans.trans_raw("artist", &answer);
        let url = gallery_entity.url().url();
        let preview = link(&preview, &gallery_entity.title_jp.unwrap_or(gallery_entity.title));
        let score = poll.score * 100.;
        let rank = poll.rank().await? * 100.;

        let text = format!(
            "{mention} {result}，答案是 {artist}（{answer}）\n回答情况：{stat_success}/{stat_total}\n地址：{url}\n预览：{preview}\n评分：{score:.2}（{rank:.2}%）",
        );

        bot.edit_message_caption(message.chat.id, message.id).caption(text).await?;
    }
    Ok(())
}

async fn callback_vote_for_poll(
    bot: Bot,
    query: CallbackQuery,
    limiter: RateLimiter,
    (poll, option): (i64, i32),
) -> Result<()> {
    if let Some(d) = limiter.insert(query.from.id) {
        bot.answer_callback_query(query.id)
            .text(format!("操作频率过高，请等待 {} 秒后再试", d.as_secs()))
            .show_alert(true)
            .await?;
        return Ok(());
    }

    info!("用户投票：[{}] {} = {}", query.from.id, poll, option);

    let old_votes = PollEntity::get_vote(poll).await?;
    VoteEntity::create(query.from.id.0, poll, option).await?;
    let votes = PollEntity::get_vote(poll).await?;

    // 投票没有变化时不要更新，不然会报错 MessageNotModified
    if old_votes != votes {
        let score = PollEntity::update_score(poll).await?;
        info!("更新分数：{} = {}", poll, score);
        let sum = votes.iter().sum::<i32>();
        let keyboard = poll_keyboard(poll, &votes);
        let text = format!("当前 {} 人投票，{:.2} 分", sum, score * 100.);

        if let Some(message) = query.message {
            bot.edit_message_text(message.chat.id, message.id, text).reply_markup(keyboard).await?;
        }
    }

    bot.answer_callback_query(query.id).text("投票成功").await?;

    Ok(())
}

async fn callback_change_page(
    bot: Bot,
    query: CallbackQuery,
    callback: CallbackData,
    cfg: Config,
) -> Result<()> {
    let (from, to, offset) = match callback {
        CallbackData::PrevPage(from, to, offset) => (from, to, offset - 1),
        CallbackData::NextPage(from, to, offset) => (from, to, offset + 1),
        _ => unreachable!(),
    };
    let text = cmd_best_text(from, to, offset, cfg.telegram.channel_id).await?;
    let keyboard = cmd_best_keyboard(from, to, offset);

    if let Some(message) = query.message {
        bot.edit_message_text(message.chat.id, message.id, text)
            .reply_markup(keyboard)
            .disable_web_page_preview(true)
            .await?;
    }

    Ok(())
}

async fn callback_random_another(
    bot: Bot,
    query: CallbackQuery,
    cfg: Config,
    tags_str: String,
) -> Result<()> {
    let message = query.message.context("消息过旧")?;
    info!("{}: <- random another {}", query.from.id, tags_str);

    let tags: Vec<String> = tags_str.split_whitespace().map(|s| s.to_string()).collect();

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
            
            let preview = gallery_preview_url(cfg.telegram.channel_id.clone(), gallery.id).await?;
            let url = gallery.url().url();
            
            let text = format!(
                "🎲 <b>隨機抽取結果</b>\n\n<b>{}</b>\n\n📄 <b>預覽：</b>{}\n🔗 <b>地址：</b>{}\n⭐️ <b>評分：</b>{:.2}（{:.2}%）",
                gallery.title_jp.as_ref().unwrap_or(&gallery.title),
                preview,
                url,
                score,
                rank
            );

            let keyboard = teloxide::types::InlineKeyboardMarkup::new(vec![vec![
                teloxide::types::InlineKeyboardButton::callback("🎲 再來一個本子", CallbackData::RandomAnother(tags_str).pack()),
            ]]);

            // 🌟核心修復：發送帶封面的圖片消息
            let images = ImageEntity::get_by_gallery_id(gallery.id).await?;
            if let Some(img) = images.first() {
                bot.send_photo(message.chat.id, InputFile::url(img.url().parse()?))
                    .caption(&text)
                    .reply_markup(keyboard)
                    .await?;
            } else {
                bot.send_message(message.chat.id, &text)
                    .reply_markup(keyboard)
                    .await?;
            }
            
            bot.answer_callback_query(query.id).await?;
        }
        None => {
            bot.answer_callback_query(query.id)
                .text("沒有找到更多匹配的本子了，請換個關鍵詞試試")
                .show_alert(true)
                .await?;
        }
    }
    Ok(())
}
