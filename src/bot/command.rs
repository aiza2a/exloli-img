use teloxide::utils::command::BotCommands;

// NOTE: 此處必須實現 Clone，否則不滿足 dptree 的 Injectable 約束
#[derive(BotCommands, Clone, PartialEq, Debug)]
#[command(rename_rule = "lowercase")]
pub enum AdminCommand {
    // 改為 String 以便手動檢查參數是否為空
    #[command(description = "根據 E 站 URL 強制上傳畫廊 (管理員)")]
    Upload(String),
    #[command(description = "刪除所回覆的畫廊 (軟刪除)")]
    Delete,
    #[command(description = "完全刪除所回覆的畫廊 (硬刪除/修復缺頁)")]
    Erase,
    #[command(description = "檢測並補檔 80 分以上或最近兩個月的本子的預覽")]
    ReCheck,
}

#[derive(BotCommands, Clone, PartialEq, Debug)]
#[command(rename_rule = "lowercase")]
pub enum PublicCommand {
    // 改為 String 以便發送使用說明
    #[command(description = "根據 E 站 URL 上傳已收錄的畫廊")]
    Upload(String),
    #[command(description = "根據消息 URL 更新指定畫廊")]
    Update(String),
    #[command(description = "根據 E 站 URL 查詢畫廊信息")]
    Query(String),
    // 移除 split 解析，改為手動解析以提供更好報錯
    #[command(description = "查詢排行榜 (用法: /best 30 0)")]
    Best(String),
    #[command(description = "猜本子遊戲")]
    Challenge,
    #[command(description = "Ping~")]
    Ping,
    #[command(description = "顯示幫助信息")]
    Help,
}
