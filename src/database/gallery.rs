use anyhow::Result;
use chrono::NaiveDate;

use super::db::DB;
use crate::ehentai::GalleryInfo;

#[derive(Debug, Clone, Default)]
pub struct GalleryEntity {
    pub id: i32,
    pub token: String,
    pub title: String,
    pub title_jp: Option<String>,
    pub tags: sqlx::types::Json<Vec<String>>,
    pub view: i32,
    pub posted: Option<NaiveDate>,
    pub deleted: bool,
}

impl GalleryEntity {
    pub async fn create(g: &impl GalleryInfo) -> Result<i32> {
        let pool = DB.get().unwrap();
        let rec = sqlx::query!(
            "INSERT OR REPLACE INTO gallery (id, token, title, title_jp, tags, view, posted) VALUES (?, ?, ?, ?, ?, ?, ?)",
            g.url().id(),
            g.url().token(),
            g.title(),
            g.title_jp(),
            sqlx::types::Json(g.tags()),
            0,
            g.posted(),
        )
        .execute(pool)
        .await?;
        Ok(rec.last_insert_rowid() as i32)
    }

    pub async fn get(id: i32) -> Result<Option<Self>> {
        let pool = DB.get().unwrap();
        let rec = sqlx::query_as!(Self, "SELECT * FROM gallery WHERE id = ?", id)
            .fetch_optional(pool)
            .await?;
        Ok(rec)
    }

    pub async fn check(id: i32) -> Result<bool> {
        let pool = DB.get().unwrap();
        let rec = sqlx::query!("SELECT count(*) as count FROM gallery WHERE id = ?", id)
            .fetch_one(pool)
            .await?;
        Ok(rec.count > 0)
    }

    pub async fn update_deleted(id: i32, deleted: bool) -> Result<()> {
        let pool = DB.get().unwrap();
        sqlx::query!("UPDATE gallery SET deleted = ? WHERE id = ?", deleted, id)
            .execute(pool)
            .await?;
        Ok(())
    }

    pub async fn delete(id: i32) -> Result<()> {
        let pool = DB.get().unwrap();
        sqlx::query!("DELETE FROM gallery WHERE id = ?", id)
            .execute(pool)
            .await?;
        Ok(())
    }

    pub async fn list(
        start: NaiveDate,
        end: NaiveDate,
        limit: i32,
        page: i32,
    ) -> Result<Vec<(f32, String, i32)>> {
        let pool = DB.get().unwrap();
        let offset = page * limit;
        let rec = sqlx::query!(
            r#"
            SELECT T1.score, T2.title, T2.title_jp, T2.id
            FROM poll AS T1
            LEFT JOIN gallery AS T2 ON T1.gallery_id = T2.id
            WHERE T2.posted BETWEEN ? AND ? AND T2.deleted = FALSE
            ORDER BY T1.score DESC
            LIMIT ? OFFSET ?
            "#,
            start,
            end,
            limit,
            offset
        )
        .fetch_all(pool)
        .await?;
        Ok(rec
            .into_iter()
            .map(|r| (r.score, r.title_jp.unwrap_or(r.title), r.id as i32))
            .collect())
    }

    /// 獲取所有掃描過的畫廊（用於舊版重傳邏輯）
    pub async fn list_scans() -> Result<Vec<Self>> {
        let pool = DB.get().unwrap();
        let rec = sqlx::query_as!(
            Self,
            "SELECT * FROM gallery WHERE deleted = FALSE ORDER BY id DESC LIMIT 100"
        )
        .fetch_all(pool)
        .await?;
        Ok(rec)
    }

    pub async fn get_by_msg(id: i32) -> Result<Option<Self>> {
        let pool = DB.get().unwrap();
        let rec = sqlx::query_as!(
            Self,
            r#"
            SELECT T2.* FROM message AS T1
            LEFT JOIN gallery AS T2 ON T1.gallery_id = T2.id
            WHERE T1.id = ?
            "#,
            id
        )
        .fetch_optional(pool)
        .await?;
        Ok(rec)
    }

    // ========================================================================
    // 🔥 第一階段新增功能 (Random & Stats)
    // ========================================================================

    /// 隨機獲取一個未刪除的畫廊
    pub async fn get_random() -> Result<Option<Self>> {
        let pool = DB.get().ok_or(anyhow::anyhow!("資料庫未連接"))?;
        let rec = sqlx::query_as!(
            Self,
            "SELECT * FROM gallery WHERE deleted = FALSE ORDER BY RANDOM() LIMIT 1"
        )
        .fetch_optional(pool)
        .await?;
        Ok(rec)
    }

    /// 統計畫廊總數
    pub async fn count() -> Result<i64> {
        let pool = DB.get().ok_or(anyhow::anyhow!("資料庫未連接"))?;
        let rec = sqlx::query!("SELECT COUNT(*) as count FROM gallery WHERE deleted = FALSE")
            .fetch_one(pool)
            .await?;
        Ok(rec.count as i64)
    }
}

impl GalleryInfo for GalleryEntity {
    fn url(&self) -> crate::ehentai::EhGalleryUrl {
        crate::ehentai::EhGalleryUrl::new(self.id, &self.token)
    }

    fn title(&self) -> String {
        self.title.clone()
    }

    fn title_jp(&self) -> String {
        self.title_jp.clone().unwrap_or(self.title.clone())
    }

    fn tags(&self) -> Vec<String> {
        self.tags.0.clone()
    }

    fn posted(&self) -> Option<NaiveDate> {
        self.posted
    }
}
