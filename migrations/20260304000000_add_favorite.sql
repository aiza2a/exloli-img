CREATE TABLE IF NOT EXISTS favorite (
    user_id BIGINT NOT NULL,
    gallery_id INTEGER NOT NULL,
    PRIMARY KEY (user_id, gallery_id)
);
