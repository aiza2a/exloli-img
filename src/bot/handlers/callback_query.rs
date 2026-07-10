use anyhow::{Context, Result};
use teloxide::dispatching::DpHandlerDescription;
use teloxide::dptree::case;
use teloxide::prelude::*;
use tracing::info;

use super::utils::gallery_preview_url;
use crate::bot::handlers::{cmd_best_keyboard, cmd_best_text, poll_keyboard};
use crate::bot::utils::{CallbackData, ChallengeLocker, RateLimiter};
use crate::bot::Bot;
use crate::config::Config;
use crate::database::{ChallengeHistory, GalleryEntity, ImageEntity, PollEntity, VoteEntity};
use crate::ehentai::GalleryInfo;
use crate::tags::EhTagTransDB;
use teloxide::types::{InputFile, ParseMode};
use teloxide::utils::html::{escape, link, user_mention};

pub fn callback_query_handler() -> Handler<'static, DependencyMap, Result<()>, DpHandlerDescription>
{
    dptree::entry()
        .branch(case![CallbackData::VoteForPoll(poll, option)].endpoint(callback_vote_for_poll))
        .branch(case![CallbackData::Challenge(id, artist)].endpoint(callback_challenge))
        .branch(case![CallbackData::RandomAnother(tags)].endpoint(callback_random_another))
        .branch(case![CallbackData::FavToggle(id)].endpoint(callback_fav_toggle))
        .branch(case![CallbackData::FavPage(page)].endpoint(callback_fav_page))
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
        let preview =
            link(&preview, &escape(&gallery_entity.title_jp.unwrap_or(gallery_entity.title)));
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

    if old_votes != votes {
        let score = PollEntity::update_score(poll).await?;
        info!("更新分数：{} = {}", poll, score);
        let sum = votes.iter().sum::<i32>();

        // 🌟 透過 Telegram 回覆鏈追溯當前的精確畫廊 ID，確保收藏按鈕不丟失
        let mut gallery_id = poll as i32; // 默認降級
        if let Some(message) = &query.message {
            if let Some(reply_to) = message.reply_to_message() {
                if let Some(fwd_msg_id) = reply_to.forward_from_message_id() {
                    if let Ok(Some(g)) = GalleryEntity::get_by_msg(fwd_msg_id).await {
                        gallery_id = g.id;
                    }
                }
            }
        }

        // 🌟 生成帶有收藏按鈕的新鍵盤
        let fav_count =
            crate::database::FavoriteEntity::count_by_gallery(gallery_id).await.unwrap_or(0);
        let keyboard = poll_keyboard(poll, &votes, gallery_id, fav_count);
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
        _ => return Ok(()), // 🌟 修復：如果不是翻頁按鈕，直接優雅忽略，絕不崩潰
    };

    let text = cmd_best_text(from, to, offset, cfg.telegram.channel_id).await?;
    let keyboard = cmd_best_keyboard(from, to, offset);

    // 🌟 先消除按鈕加載狀態
    let _ = bot.answer_callback_query(query.id.clone()).await;

    if let Some(message) = query.message {
        // 🌟 忽略錯誤，並補上 parse_mode
        let _ = bot
            .edit_message_text(message.chat.id, message.id, text)
            .reply_markup(keyboard)
            .disable_web_page_preview(true)
            .parse_mode(ParseMode::Html)
            .await;
    }

    Ok(())
}

// 同樣注入 trans: EhTagTransDB
// 同樣注入 trans: EhTagTransDB
async fn callback_random_another(
    bot: Bot,
    query: CallbackQuery,
    cfg: Config,
    trans: EhTagTransDB,
    tags_str: String,
) -> Result<()> {
    let message = query.message.context("消息过旧")?;
    info!("{}: <- random another {}", query.from.id, tags_str);

    let tags: Vec<String> = tags_str.split_whitespace().map(|s| s.to_string()).collect();

    // 🌟核心：獲取翻譯陣列
    let tags_conditions: Vec<Vec<String>> = tags.iter().map(|t| trans.search_raw_tags(t)).collect();

    let gallery = if tags_conditions.is_empty() {
        GalleryEntity::get_random().await?
    } else {
        GalleryEntity::get_random_with_tags(&tags_conditions).await?
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

            // 🌟 這是真正的 text 構建，已經去掉了那個報錯的佔位符
            let text = format!(
                "🎲 <b>隨機抽取結果</b>\n\n<b>{}</b>\n\n📄 <b>預覽：</b>{}\n🔗 <b>地址：</b>{}\n⭐️ <b>評分：</b>{:.2}（{:.2}%）",
                escape(gallery.title_jp.as_ref().unwrap_or(&gallery.title)),
                preview,
                url,
                score,
                rank
            );

            // 🌟 動態獲取人數並把收藏按鈕加到鍵盤裡
            let fav_count =
                crate::database::FavoriteEntity::count_by_gallery(gallery.id).await.unwrap_or(0);
            let fav_text = if fav_count > 0 {
                format!("⭐ 收藏 ({})", fav_count)
            } else {
                "⭐ 收藏".to_string()
            };
            let fav_btn = teloxide::types::InlineKeyboardButton::callback(
                fav_text,
                CallbackData::FavToggle(gallery.id).pack(),
            );

            let keyboard = teloxide::types::InlineKeyboardMarkup::new(vec![vec![
                teloxide::types::InlineKeyboardButton::callback(
                    "🎲 再來一個本子",
                    CallbackData::RandomAnother(tags_str).pack(),
                ),
                fav_btn, // 🌟 與再來一本並排顯示
            ]]);

            let images = ImageEntity::get_by_gallery_id(gallery.id).await?;
            if let Some(img) = images.first() {
                bot.send_photo(message.chat.id, InputFile::url(img.url().parse()?))
                    .caption(&text)
                    .parse_mode(ParseMode::Html)
                    .reply_markup(keyboard)
                    .await?;
            } else {
                bot.send_message(message.chat.id, &text)
                    .parse_mode(ParseMode::Html)
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

async fn callback_fav_toggle(bot: Bot, query: CallbackQuery, id: i32) -> Result<()> {
    let user_id = query.from.id.0 as i64;
    let added = crate::database::FavoriteEntity::toggle(user_id, id).await?;

    // 全局彈窗
    let alert_msg =
        if added { "⭐ 已加入個人收藏夾！" } else { "❌ 已從收藏夾移除！" };
    bot.answer_callback_query(query.id.clone()).text(alert_msg).show_alert(false).await?;

    // 🌟 獲取最新人數
    let fav_count = crate::database::FavoriteEntity::count_by_gallery(id).await.unwrap_or(0);

    if let Some(message) = &query.message {
        if let Some(markup) = message.reply_markup() {
            let mut new_inline_keyboard = markup.inline_keyboard.clone();
            let target_pack = CallbackData::FavToggle(id).pack();
            let mut found = false;

            for row in &mut new_inline_keyboard {
                for button in row {
                    // 🌟 修复：使用 if let 模式匹配解构 Enum，安全提取内部的字符串
                    if let teloxide::types::InlineKeyboardButtonKind::CallbackData(ref data) =
                        button.kind
                    {
                        if data == &target_pack {
                            let old_text = button.text.clone();
                            // 核心：如果是私聊，字会变成"✅ 已收藏"；如果是群聊/频道，维持原字不变！
                            let base_text = if message.chat.is_private() {
                                if added {
                                    "✅ 已收藏"
                                } else {
                                    "⭐ 收藏"
                                }
                            } else {
                                if old_text.starts_with("⭐ 收藏本檔案") {
                                    "⭐ 收藏本檔案"
                                } else {
                                    "⭐ 收藏"
                                }
                            };

                            // 拼接人数
                            button.text = if fav_count > 0 {
                                format!("{} ({})", base_text, fav_count)
                            } else {
                                base_text.to_string()
                            };

                            found = true;
                            break;
                        }
                    }
                }
                if found {
                    break;
                }
            }

            if found {
                let new_markup = teloxide::types::InlineKeyboardMarkup::new(new_inline_keyboard);
                let _ = bot
                    .edit_message_reply_markup(message.chat.id, message.id)
                    .reply_markup(new_markup)
                    .await;
            }
        }
    }
    Ok(())
}

async fn callback_fav_page(bot: Bot, query: CallbackQuery, page: i32, cfg: Config) -> Result<()> {
    let message = query.message.context("消息过旧")?;
    let user_id = query.from.id.0 as i64;

    let text =
        crate::bot::handlers::fav_text(user_id, page, cfg.telegram.channel_id.clone()).await?;
    let count = crate::database::FavoriteEntity::count(user_id).await?;
    let keyboard = crate::bot::handlers::fav_keyboard(page, count);

    // 🌟 先消除加載動畫
    let _ = bot.answer_callback_query(query.id).await;

    // 🌟 忽略內容未修改引發的報錯
    let _ = bot
        .edit_message_text(message.chat.id, message.id, text)
        .reply_markup(keyboard)
        .parse_mode(teloxide::types::ParseMode::Html)
        .disable_web_page_preview(true)
        .await;

    Ok(())
}
