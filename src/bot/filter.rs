use teloxide::dispatching::DpHandlerDescription;
use teloxide::prelude::*;
use teloxide::types::{ChatKind, ChatMemberKind, Recipient};

use super::utils::CallbackData;
use super::Bot;
use crate::config::Config;

pub fn filter_admin_msg<Output>() -> Handler<'static, DependencyMap, Output, DpHandlerDescription>
where
    Output: Send + Sync + 'static,
{
    dptree::filter_async(|message: Message, bot: Bot, cfg: Config| async move {
        let Some(user) = message.from() else {
            return false;
        };
        bot.get_chat_member(cfg.telegram.group_id, user.id)
            .await
            .map(|member| {
                matches!(member.kind, ChatMemberKind::Administrator(_) | ChatMemberKind::Owner(_))
            })
            .unwrap_or_default()
    })
}

pub fn filter_public_command_rate<Output>() -> Handler<'static, DependencyMap, Output, DpHandlerDescription>
where
    Output: Send + Sync + 'static,
{
    dptree::filter_async(|message: Message, bot: Bot, limiter: super::utils::RateLimiter| async move {
        let Some(user) = message.from() else {
            return false;
        };
        let Some(wait) = limiter.insert(user.id) else {
            return true;
        };

        let _ = bot
            .send_message(
                message.chat.id,
                format!("操作频率过高，请等待 {} 秒后再试", wait.as_secs().max(1)),
            )
            .reply_to_message_id(message.id)
            .await;
        false
    })
}

pub fn filter_channel_msg<Output>() -> Handler<'static, DependencyMap, Output, DpHandlerDescription>
where
    Output: Send + Sync + 'static,
{
    dptree::filter(|message: Message, cfg: Config| {
        message.from().map(|u| u.id.0 == 777000).unwrap_or_default()
            && message.text().map(|s| s.contains("來 源")).unwrap_or_default()
            && cfg.telegram.group_id == message.chat.id
    })
}

pub fn filter_callbackdata<Output>() -> Handler<'static, DependencyMap, Output, DpHandlerDescription>
where
    Output: Send + Sync + 'static,
{
    dptree::filter_map(|callback: CallbackQuery| {
        callback.data.and_then(|s| CallbackData::unpack(&s))
    })
}

pub fn filter_member<C, Output>(
    chat_id: C,
    status: ChatMemberKind,
) -> Handler<'static, DependencyMap, Output, DpHandlerDescription>
where
    Output: Send + Sync + 'static,
    C: Send + Sync + Into<Recipient>,
{
    let chat_id = chat_id.into();
    dptree::filter_async(move |message: Message, bot: Bot| {
        let chat_id = chat_id.clone();
        let status = status.clone();
        async move {
            if let Some(user) = message.from() {
                if let Ok(member) = bot.get_chat_member(chat_id, user.id).await {
                    if member.kind == status {
                        return true;
                    }
                }
            }
            false
        }
    })
}

pub fn filter_private_chat<Output>() -> Handler<'static, DependencyMap, Output, DpHandlerDescription>
where
    Output: Send + Sync + 'static,
{
    dptree::filter(|message: Message| matches!(message.chat.kind, ChatKind::Private(_)))
}
