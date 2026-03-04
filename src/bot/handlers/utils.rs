use anyhow::{anyhow, Result};
use chrono::{Duration, Utc};
use reqwest::Url;
use teloxide::prelude::*;
use teloxide::types::{
    InlineKeyboardButton, InlineKeyboardButtonKind, InlineKeyboardMarkup, MessageId, Recipient,
};
use teloxide::utils::html::{escape, link};

use crate::bot::utils::CallbackData;
use crate::database::{ChallengeView, GalleryEntity, MessageEntity, TelegraphEntity};
use crate::tags::EhTagTransDB;

pub fn cmd_challenge_keyboard(
    id: i64,
    challenge: &[ChallengeView],
    trans: &EhTagTransDB,
) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(challenge.iter().map(|g| {
        vec![InlineKeyboardButton::callback(
            format!("{}（{}）", trans.trans_raw("artist", &g.artist), &g.artist),
            CallbackData::Challenge(id, g.artist.clone()).pack(),
        )]
    }))
}

pub async fn cmd_best_text(
    day_a: i32,
    day_b: i32,
    offset: i32,
    channel: Recipient,
) -> Result<String> {
    // 自動排序天數，確保 max_days 是較大的數字（更久以前），min_days 是較小的數字（較近期）
    let max_days = day_a.max(day_b);
    let min_days = day_a.min(day_b);

    // from_date 必須是較早的日期
    let from_date = Utc::now().date_naive() - Duration::days(max_days as i64);
    // to_date 必須是較晚的日期
    let to_date = Utc::now().date_naive() - Duration::days(min_days as i64);

    let mut text = format!("最近 {} ~ {} 天的本子排名（{}）", min_days, max_days, offset);

    for (score, title, gid) in GalleryEntity::list(from_date, to_date, 20, offset).await? {
        let url = gallery_preview_url(channel.clone(), gid).await?;
        text.push_str(&format!("\n<code>{:.2}</code> - {}", score * 100., link(&url, &title)));
    }

    Ok(text)
}

pub fn cmd_best_keyboard(from: i32, to: i32, offset: i32) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("<", CallbackData::PrevPage(from, to, offset).pack()),
        InlineKeyboardButton::callback(">", CallbackData::NextPage(from, to, offset).pack()),
    ]])
}

pub fn url_of(channel: Recipient, id: i32) -> Url {
    match channel {
        Recipient::Id(chat_id) => Message::url_of(chat_id, None, MessageId(id)).unwrap(),
        Recipient::ChannelUsername(username) => {
            Message::url_of(ChatId(-1000000000000), Some(&username[1..]), MessageId(id)).unwrap()
        }
    }
}

pub fn poll_keyboard(poll_id: i64, votes: &[i32; 5]) -> InlineKeyboardMarkup {
    let sum = votes.iter().sum::<i32>();
    let votes: Box<dyn Iterator<Item = f32>> = if sum == 0 {
        Box::new([0.].iter().cloned().cycle())
    } else {
        Box::new(votes.iter().map(|&i| i as f32 / sum as f32 * 100.))
    };

    let options = ["我瞎了", "不咋样", "还行吧", "不错哦", "太棒了"]
        .iter()
        .zip(votes)
        .enumerate()
        .map(|(idx, (name, vote))| {
            vec![InlineKeyboardButton::new(
                format!("{:.0}% {}", vote, name),
                InlineKeyboardButtonKind::CallbackData(
                    CallbackData::VoteForPoll(poll_id, (idx + 1) as i32).pack(),
                ),
            )]
        })
        .collect::<Vec<_>>();

    InlineKeyboardMarkup::new(options)
}

pub async fn gallery_preview_url(channel_id: Recipient, gallery_id: i32) -> Result<String> {
    if let Some(msg) = MessageEntity::get_by_gallery(gallery_id).await? {
        return Ok(url_of(channel_id, msg.id).to_string());
    }
    if let Some(telehraph) = TelegraphEntity::get(gallery_id).await? {
        return Ok(telehraph.url);
    }
    Err(anyhow!("找不到画廊"))
}
pub async fn fav_text(user_id: i64, page: i32, channel: Recipient) -> Result<String> {
    let limit = 15;
    let count = FavoriteEntity::count(user_id).await?;
    if count == 0 {
        return Ok("<b>📚 您的個人收藏夾</b>\n\n您目前還沒有收藏任何檔案哦！\n點擊畫廊底部的 <b>[⭐ 收藏]</b> 按鈕即可加入。".to_string());
    }
    
    let total_pages = (count + limit - 1) / limit;
    let current_page = page.clamp(0, total_pages - 1);
    let mut text = format!("📚 <b>您的個人收藏夾</b> (第 {}/{} 頁，共 {} 本)\n\n", current_page + 1, total_pages, count);

    let list = FavoriteEntity::list(user_id, limit, current_page).await?;
    for (gid, title, score) in list {
        let url = gallery_preview_url(channel.clone(), gid).await?;
        text.push_str(&format!("<code>{:.2}</code> - {}\n", score * 100., link(&url, &escape(&title))));
    }
    Ok(text)
}

pub fn fav_keyboard(page: i32, total: i32) -> InlineKeyboardMarkup {
    let limit = 15;
    let total_pages = (total + limit - 1) / limit;
    let mut row = vec![];
    if page > 0 { row.push(InlineKeyboardButton::callback("<", CallbackData::FavPage(page - 1).pack())); }
    if page < total_pages - 1 { row.push(InlineKeyboardButton::callback(">", CallbackData::FavPage(page + 1).pack())); }
    InlineKeyboardMarkup::new(if row.is_empty() { vec![] } else { vec![row] })
}
