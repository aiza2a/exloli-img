use super::db::DB;
use sqlx::sqlite::SqliteQueryResult;
use sqlx::Result;

pub struct BadImageEntity;

impl BadImageEntity {
    pub async fn is_bad(hash: &str) -> Result<Option<i32>> {
        let res: Option<(i32,)> = sqlx::query_as("SELECT type FROM bad_image WHERE hash = ?")
            .bind(hash)
            .fetch_optional(&*DB)
            .await?;
        Ok(res.map(|r| r.0))
    }

    pub async fn mark(hash: &str, img_type: i32) -> Result<SqliteQueryResult> {
        sqlx::query("INSERT OR REPLACE INTO bad_image (hash, type) VALUES (?, ?)")
            .bind(hash)
            .bind(img_type)
            .execute(&*DB)
            .await
    }
}
