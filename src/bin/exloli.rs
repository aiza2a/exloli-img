// src/bin/exloli.rs 完整文件内容
use std::env;

use anyhow::Result;
use exloli_next::bot::start_dispatcher;
use exloli_next::config::{Config, CHANNEL_ID};
use exloli_next::ehentai::EhClient;
use exloli_next::tags::EhTagTransDB;
use exloli_next::uploader::ExloliUploader;
use teloxide::prelude::*;
use teloxide::types::{ParseMode, Recipient, BotCommandScope};
use teloxide::utils::command::BotCommands;
// 引入指令定義
use exloli_next::bot::command::{AdminCommand, PublicCommand};

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::new("./config.toml")?;
    CHANNEL_ID.set(config.telegram.channel_id.to_string()).unwrap();

    // NOTE: 全局數據庫連接需要用這個變量初始化
    env::set_var("DATABASE_URL", &config.database_url);
    env::set_var("RUST_LOG", &config.log_level);

    tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init()
        .unwrap();

    let trans = EhTagTransDB::new(&config.exhentai.trans_file);
    let ehentai = EhClient::new(&config.exhentai.cookie).await?;
    
    let bot = Bot::new(&config.telegram.token)
        .throttle(Default::default())
        .parse_mode(ParseMode::Html)
        .cache_me();

    // ========================================================
    // 🔥🔥🔥 註冊指令菜單 🔥🔥🔥
    // ========================================================
    
    tracing::info!("正在向 Telegram 註冊指令列表...");

    // 1. 為所有用戶註冊公共指令
    bot.set_my_commands(PublicCommand::bot_commands())
        .scope(BotCommandScope::Default)
        .await?;

    // 2. 為管理員群組註冊完整指令 (包含 AdminCommand)
    // 這裡修正了 ChatId 和 Recipient 的轉換邏輯
    let admin_chat_id = config.telegram.group_id;
    if !admin_chat_id.is_invalid() {
        let mut full_commands = PublicCommand::bot_commands();
        full_commands.extend(AdminCommand::bot_commands());
        
        // 將 ChatId 轉換為 Recipient::Id
        bot.set_my_commands(full_commands)
            .scope(BotCommandScope::Chat { 
                chat_id: Recipient::Id(admin_chat_id) 
            })
            .await?;
        tracing::info!("已為管理群組 {} 註冊管理員指令。", admin_chat_id);
    }
    
    tracing::info!("指令註冊完成！");
    // ========================================================

    let uploader =
        ExloliUploader::new(config.clone(), ehentai.clone(), bot.clone(), trans.clone()).await?;

    let t1 = {
        let uploader = uploader.clone();
        tokio::spawn(async move { uploader.start().await })
    };
    let t2 = {
        let trans = trans.clone();
        tokio::spawn(async move { start_dispatcher(config, uploader, bot, trans).await })
    };
    let t3 = tokio::spawn(async move { trans.start().await });

    tokio::try_join!(t1, t2, t3)?;

    Ok(())
}
