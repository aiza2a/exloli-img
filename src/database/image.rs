use sqlx::sqlite::SqliteQueryResult;
use sqlx::Result;

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
    pub async fn create(id: u32, hash: &str, url: &str) -> Result<SqliteQueryResult> {
        sqlx::query("INSERT OR IGNORE INTO image (id, hash, url) VALUES (?, ?, ?)")
            .bind(id)
            .bind(hash)
            .bind(url)
            .execute(&*DB)
            .await
    }

    pub async fn get_by_hash(hash: &str) -> Result<Option<Self>> {
        sqlx::query_as::<_, Self>("SELECT id, hash, url FROM image WHERE hash = ?")
            .bind(hash)
            .fetch_optional(&*DB)
            .await
    }

    pub async fn get_by_gallery_id(gallery_id: i32) -> Result<Vec<Self>> {
        sqlx::query_as::<_, Self>(
            r#"SELECT image.id, image.hash, image.url
            FROM image JOIN page ON page.image_id = image.id
            WHERE page.gallery_id = ? ORDER BY page.page"#,
        )
        .bind(gallery_id)
        .fetch_all(&*DB)
        .await
    }

    pub fn url(&self) -> String {
        if self.url.starts_with("/file/") {
            format!("https://telegra.ph{}", self.url)
        } else {
            self.url.clone()
        }
    }

    pub async fn count() -> Result<i32> {
        let res: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM image").fetch_one(&*DB).await?;
        Ok(res.0 as i32)
    }
}

impl PageEntity {
    pub async fn create(gallery_id: i32, page: i32, image_id: u32) -> Result<SqliteQueryResult> {
        sqlx::query("INSERT OR IGNORE INTO page (gallery_id, page, image_id) VALUES (?, ?, ?)")
            .bind(gallery_id)
            .bind(page)
            .bind(image_id)
            .execute(&*DB)
            .await
    }
    pub async fn count(gallery_id: i32) -> Result<i32> {
        let res: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM page WHERE gallery_id = ?")
            .bind(gallery_id)
            .fetch_one(&*DB)
            .await?;
        Ok(res.0 as i32)
    }
}
