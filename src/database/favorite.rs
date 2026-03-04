use sqlx::Result;
use super::db::DB;

pub struct FavoriteEntity;

impl FavoriteEntity {
    // 切換收藏狀態（有則刪除，無則添加）
    pub async fn toggle(user_id: i64, gallery_id: i32) -> Result<bool> {
        let exists: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM favorite WHERE user_id = ? AND gallery_id = ?")
            .bind(user_id).bind(gallery_id).fetch_optional(&*DB).await?;
        
        if exists.is_some() {
            sqlx::query("DELETE FROM favorite WHERE user_id = ? AND gallery_id = ?")
                .bind(user_id).bind(gallery_id).execute(&*DB).await?;
            Ok(false) // 返回 false 代表已取消收藏
        } else {
            sqlx::query("INSERT INTO favorite (user_id, gallery_id) VALUES (?, ?)")
                .bind(user_id).bind(gallery_id).execute(&*DB).await?;
            Ok(true) // 返回 true 代表已加入收藏
        }
    }

    // 獲取收藏列表 (聯表查詢獲取標題和分數)
    pub async fn list(user_id: i64, limit: i32, page: i32) -> Result<Vec<(i32, String, f32)>> {
        let offset = page * limit;
        #[derive(sqlx::FromRow)]
        struct Row { id: i32, title: String, score: f64 }

        let records = sqlx::query_as::<_, Row>(
            r#"SELECT gallery.id, COALESCE(gallery.title_jp, gallery.title) as title, IFNULL(poll.score, 0.0) as score
            FROM favorite
            JOIN gallery ON favorite.gallery_id = gallery.id
            LEFT JOIN poll ON favorite.gallery_id = poll.gallery_id
            WHERE favorite.user_id = ? AND gallery.deleted = FALSE
            ORDER BY favorite.rowid DESC LIMIT ? OFFSET ?"#
        )
        .bind(user_id).bind(limit).bind(offset).fetch_all(&*DB).await?;

        Ok(records.into_iter().map(|r| (r.id, r.title, r.score as f32)).collect())
    }

    pub async fn count(user_id: i64) -> Result<i32> {
        let res: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM favorite WHERE user_id = ?")
            .bind(user_id).fetch_one(&*DB).await?;
        Ok(res.0 as i32)
    }
    // 🌟 獲取特定畫廊的收藏總數
    pub async fn count_by_gallery(gallery_id: i32) -> Result<i32> {
        let res: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM favorite WHERE gallery_id = ?")
            .bind(gallery_id).fetch_one(&*DB).await?;
        Ok(res.0 as i32)
    }
}
