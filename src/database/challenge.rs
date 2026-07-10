use chrono::prelude::*;
use sqlx::prelude::*;
use sqlx::sqlite::SqliteQueryResult;
use sqlx::Result;

use super::db::DB;

#[derive(FromRow, Clone)]
pub struct ChallengeView {
    pub id: i32,
    pub token: String,
    pub page: i32,
    pub artist: String,
    pub image_id: i32,
    pub url: String,
    pub score: f32,
}

pub struct ChallengeHistory {
    pub id: i32,
    pub user_id: i64,
    pub gallery_id: i32,
    pub page: i32,
    pub success: bool,
    pub answer_time: NaiveDateTime,
    pub chat_id: i64,
}

impl ChallengeView {
    pub async fn get_random() -> Result<Vec<Self>> {
        // 🌟 修复：去掉感叹号，改为 query_as::<_, Self>，并去掉了内部为了宏校验而写的 as "id: i32" 等类型强转
        sqlx::query_as::<_, Self>(
            r#"
            SELECT
                id,
                token,
                page,
                artist,
                image_id,
                url,
                score
            FROM (
                -- 此处使用 group by 嵌套 random，因为默认情况下 group by 只会显示每组的第一个结果
                SELECT * FROM (
                    SELECT * FROM challenge_view
                    WHERE score > 0.8 AND image_id NOT IN (
                        SELECT image_id FROM page GROUP BY gallery_id HAVING page = MAX(page)
                        UNION
                        SELECT image_id FROM page GROUP BY gallery_id HAVING page = 1
                        -- 🌟 排除被标记为广告的图片
                        UNION
                        SELECT id FROM image WHERE hash IN (SELECT hash FROM bad_image)
                    ) ORDER BY random() LIMIT 500
                ) GROUP BY artist
            ) ORDER BY random() LIMIT 4"#,
        )
        .fetch_all(&*DB)
        .await
    }
}

impl ChallengeHistory {
    pub async fn create(
        user: i64,
        gallery: i32,
        page: i32,
        success: bool,
        chat_id: i64,
    ) -> Result<SqliteQueryResult> {
        let now = Utc::now().naive_utc();
        sqlx::query!(
            "INSERT INTO challenge_history (user_id, gallery_id, page, success, answer_time, chat_id) VALUES (?, ?, ?, ?, ?, ?)",
            user,
            gallery,
            page,
            success,
            now,
            chat_id,
        )
        .execute(&*DB)
        .await
    }

    pub async fn answer_stats(user: i64, chat_id: i64) -> Result<(i32, i32)> {
        let record = sqlx::query!(
            r#"SELECT SUM(success) as "success!", COUNT(*) as "total!" FROM challenge_history WHERE user_id = ? AND chat_id = ?"#, user, chat_id,
        )
        .fetch_one(&*DB)
        .await?;
        Ok((record.success, record.total))
    }
}
