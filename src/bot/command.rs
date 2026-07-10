use teloxide::utils::command::BotCommands;

#[derive(BotCommands, Clone, PartialEq, Debug)]
#[command(rename_rule = "lowercase")]
pub enum AdminCommand {
    #[command(description = "根據 E 站 URL 強制上傳畫廊 (管理員)")]
    Upload(String),
    #[command(description = "依據 URL 或回覆消息更新畫廊元數據 (管理員)")]
    Update(String),
    #[command(description = "刪除所回覆的畫廊 (軟刪除)")]
    Delete,
    #[command(description = "完全刪除所回覆的畫廊 (硬刪除/修復缺頁)")]
    Erase,
    #[command(description = "补全缺页并重建预览 (管理員)")]
    Repair,
    #[command(description = "檢測並修復預覽鏈接")]
    ReCheck,
}

#[derive(BotCommands, Clone, PartialEq, Debug)]
#[command(rename_rule = "lowercase")]
pub enum PublicCommand {
    #[command(description = "根據 E 站 URL 上傳已收錄的畫廊")]
    Upload(String),
    #[command(description = "根據 E 站 URL 查詢畫廊信息")]
    Query(String),
    #[command(description = "查詢排行榜 (用法: /best 30 0)")]
    Best(String),
    #[command(description = "猜本子遊戲")]
    Challenge,
    #[command(description = "Ping~")]
    Ping,
    #[command(description = "顯示幫助信息")]
    Help,
    #[command(description = "隨機抽取一本本子 (用法: /random [標籤] [數量(最大为10)])")]
    Random(String),
    #[command(description = "查看您的個人收藏夾")]
    Fav,
    #[command(description = "查看 Bot 數據統計")]
    Stats,
}
