use sqlx::sqlite::SqliteQueryResult;
use sqlx::Result;
use tracing::Level;
use super::db::DB;

#[derive(sqlx::FromRow, Debug)]
pub struct PageEntity {
    pub gallery_id: i32,
    pub page: i32,
    pub image_id: u32,
}

#[derive(sqlx::FromRow, Debug, Clone)]
pub struct ImageEntity {
    pub id: u32,
    pub hash: String,
    pub url: String,
}

impl ImageEntity {
    #[tracing::instrument(level = Level::DEBUG)]
    pub async fn create(id: u32, hash: &str, url: &str) -> Result<SqliteQueryResult> {
        sqlx::query!("INSERT OR IGNORE INTO image (id, hash, url) VALUES (?, ?, ?)", id, hash, url)
            .execute(&*DB).await
    }

    pub async fn get_by_hash(hash: &str) -> Result<Option<Self>> {
        sqlx::query_as!(Self, r#"SELECT id as "id: u32", hash, url FROM image WHERE hash = ?"#, hash)
            .fetch_optional(&*DB).await
    }

    pub async fn get_by_gallery_id(gallery_id: i32) -> Result<Vec<Self>> {
        sqlx::query_as!(Self, 
            r#"SELECT image.id as "id: u32", image.hash as hash, image.url as url
            FROM image JOIN page ON page.image_id = image.id
            WHERE page.gallery_id = ? ORDER BY page.page"#,
            gallery_id,
        ).fetch_all(&*DB).await
    }

    pub fn url(&self) -> String {
        if self.url.starts_with("/file/") { format!("https://telegra.ph{}", self.url) } else { self.url.clone() }
    }

    // 🔥【第一階段新增】統計圖片
    pub async fn count_total() -> Result<i32> {
        sqlx::query_scalar!("SELECT COUNT(*) FROM image").fetch_one(&*DB).await
    }
}

impl PageEntity {
    pub async fn create(gallery_id: i32, page: i32, image_id: u32) -> Result<SqliteQueryResult> {
        sqlx::query!("INSERT OR IGNORE INTO page (gallery_id, page, image_id) VALUES (?, ?, ?)", gallery_id, page, image_id)
            .execute(&*DB).await
    }
    pub async fn count(gallery_id: i32) -> Result<i32> {
        sqlx::query_scalar!("SELECT COUNT(*) FROM page WHERE gallery_id = ?", gallery_id).fetch_one(&*DB).await
    }
}
