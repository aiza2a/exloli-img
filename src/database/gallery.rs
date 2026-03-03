use std::ops::Deref;
use chrono::prelude::*;
use chrono::Duration;
use indexmap::IndexMap;
use sqlx::database::HasValueRef;
use sqlx::error::BoxDynError;
use sqlx::prelude::*;
use sqlx::sqlite::SqliteQueryResult;
use sqlx::{Database, Result, Sqlite};

use super::db::DB;
use crate::config::CHANNEL_ID;
use crate::ehentai::EhGallery;

#[derive(Debug, Clone, Default)]
pub struct TagsEntity(pub IndexMap<String, Vec<String>>);

#[derive(Debug, Clone, FromRow)]
pub struct GalleryEntity {
    pub id: i32,
    pub token: String,
    pub title: String,
    pub title_jp: Option<String>,
    pub tags: TagsEntity,
    pub favorite: Option<i32>,
    pub pages: i32,
    pub parent: Option<i32>,
    pub deleted: bool,
    pub posted: Option<NaiveDateTime>,
}

impl GalleryEntity {
    pub async fn create(g: &EhGallery) -> Result<SqliteQueryResult> {
        let id = g.url.id();
        let token = g.url.token();
        let tags = serde_json::to_string(&g.tags).unwrap();
        let pages = g.pages.len() as i32;
        let parent = g.parent.as_ref().map(|g| g.id());
        // 改用非宏模式，避免校验
        sqlx::query("REPLACE INTO gallery (id, token, title, title_jp, tags, favorite, pages, parent, deleted, posted) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(id).bind(token).bind(&g.title).bind(&g.title_jp).bind(tags).bind(g.favorite).bind(pages).bind(parent).bind(false).bind(g.posted)
            .execute(&*DB).await
    }

    pub async fn get(id: i32) -> Result<Option<GalleryEntity>> {
        sqlx::query_as::<_, GalleryEntity>("SELECT * FROM gallery WHERE id = ? AND deleted = FALSE")
            .bind(id).fetch_optional(&*DB).await
    }

    pub async fn get_by_msg(id: i32) -> Result<Option<GalleryEntity>> {
        sqlx::query_as::<_, GalleryEntity>(
            "SELECT gallery.* FROM gallery JOIN message ON gallery.id = message.gallery_id AND message.channel_id = ? WHERE message.id = ? AND gallery.deleted = FALSE"
        ).bind(CHANNEL_ID.get().unwrap()).bind(id).fetch_optional(&*DB).await
    }

    pub async fn check(id: i32) -> Result<bool> {
        let res: Option<(i32,)> = sqlx::query_as("SELECT 1 FROM gallery WHERE id = ? LIMIT 1")
            .bind(id).fetch_optional(&*DB).await?;
        Ok(res.is_some())
    }

    pub async fn update_deleted(id: i32, deleted: bool) -> Result<SqliteQueryResult> {
        sqlx::query("UPDATE gallery SET deleted = ? WHERE id = ?").bind(deleted).bind(id).execute(&*DB).await
    }

    pub async fn delete(id: i32) -> Result<SqliteQueryResult> {
        sqlx::query("DELETE FROM gallery WHERE id = ?").bind(id).execute(&*DB).await
    }

    pub async fn list(start: NaiveDate, end: NaiveDate, limit: i32, page: i32) -> Result<Vec<(f32, String, i32)>> {
        let offset = page * limit;
        // 关键修复：显式定义匿名结构体以接收联表查询结果
        #[derive(sqlx::FromRow)]
        struct ListRow { score: f64, title: String, id: i32 }

        let records = sqlx::query_as::<_, ListRow>(
            r#"SELECT poll.score, gallery.title, gallery.id FROM gallery
            JOIN poll ON poll.gallery_id = gallery.id
            JOIN message ON message.gallery_id = gallery.id
            WHERE gallery.posted BETWEEN ? AND ? GROUP BY poll.id
            ORDER BY poll.score DESC LIMIT ? OFFSET ?"#
        ).bind(start).bind(end).bind(limit).bind(offset).fetch_all(&*DB).await?;
        
        Ok(records.into_iter().map(|x| (x.score as f32, x.title, x.id)).collect())
    }

    pub async fn list_scans() -> Result<Vec<Self>> {
        let since = Utc::now().date_naive() - Duration::days(60);
        sqlx::query_as::<_, GalleryEntity>(
            r#"SELECT gallery.* FROM gallery JOIN poll ON poll.gallery_id = gallery.id
            WHERE gallery.deleted = FALSE AND (poll.score >= 0.8 OR gallery.posted >= ?)"#,
        ).bind(since).fetch_all(&*DB).await
    }

    pub async fn get_random() -> Result<Option<Self>> {
        sqlx::query_as::<_, Self>("SELECT * FROM gallery WHERE deleted = FALSE ORDER BY RANDOM() LIMIT 1")
            .fetch_optional(&*DB).await
    }

    pub async fn get_random_with_tags(tags: &[String]) -> Result<Option<Self>> {
        // 動態構建 SQL：每個標籤都必須包含在 JSON 字符串中
        let mut query = String::from("SELECT * FROM gallery WHERE deleted = FALSE");
        for _ in tags {
            query.push_str(" AND tags LIKE ?");
        }
        query.push_str(" ORDER BY RANDOM() LIMIT 1");

        let mut q = sqlx::query_as::<_, Self>(&query);
        // 綁定模糊查詢的變量
        for tag in tags {
            q = q.bind(format!("%{}%", tag)); 
        }
        q.fetch_optional(&*DB).await
    }
    
    pub async fn count() -> Result<i32> {
        let res: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM gallery WHERE deleted = FALSE")
            .fetch_one(&*DB).await?;
        Ok(res.0 as i32)
    }
}

impl<'q> Decode<'q, Sqlite> for TagsEntity {
    fn decode(value: <Sqlite as HasValueRef<'q>>::ValueRef) -> std::result::Result<Self, BoxDynError> {
        let str = <String as Decode<Sqlite>>::decode(value)?;
        if str.is_empty() { Ok(TagsEntity(IndexMap::new())) } else { Ok(TagsEntity(serde_json::from_str(&str)?)) }
    }
}
impl Type<Sqlite> for TagsEntity {
    fn type_info() -> <Sqlite as Database>::TypeInfo { <String as Type<Sqlite>>::type_info() }
    fn compatible(ty: &<Sqlite as Database>::TypeInfo) -> bool { <String as Type<Sqlite>>::compatible(ty) }
}
impl Deref for TagsEntity {
    type Target = IndexMap<String, Vec<String>>;
    fn deref(&self) -> &Self::Target { &self.0 }
}
