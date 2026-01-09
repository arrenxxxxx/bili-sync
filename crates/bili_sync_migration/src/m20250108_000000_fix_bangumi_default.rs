use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        // SQLite 不支持直接修改列默认值，需要重建表
        db.execute_unprepared(
            "CREATE TABLE `bangumi_new` (
                `id` INTEGER PRIMARY KEY AUTOINCREMENT,
                `season_id` INTEGER NOT NULL UNIQUE,
                `media_id` INTEGER NOT NULL,
                `title` TEXT NOT NULL,
                `cover` TEXT NOT NULL,
                `evaluate` TEXT NOT NULL,
                `total` INTEGER NOT NULL,
                `is_finish` INTEGER NOT NULL,
                `season_type` INTEGER NOT NULL,
                `path` TEXT NOT NULL,
                `created_at` TEXT NOT NULL DEFAULT (CURRENT_TIMESTAMP),
                `latest_row_at` TEXT NOT NULL DEFAULT '1970-01-01 00:00:00',
                `rule` TEXT,
                `enabled` INTEGER NOT NULL DEFAULT 1
            )"
        )
        .await?;
        db.execute_unprepared(
            "INSERT INTO `bangumi_new` SELECT * FROM `bangumi`"
        )
        .await?;
        db.execute_unprepared("DROP TABLE `bangumi`").await?;
        db.execute_unprepared("ALTER TABLE `bangumi_new` RENAME TO `bangumi`").await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // 回滚：恢复原表结构（无默认值）
        let db = manager.get_connection();
        db.execute_unprepared(
            "CREATE TABLE `bangumi_old` (
                `id` INTEGER PRIMARY KEY AUTOINCREMENT,
                `season_id` INTEGER NOT NULL UNIQUE,
                `media_id` INTEGER NOT NULL,
                `title` TEXT NOT NULL,
                `cover` TEXT NOT NULL,
                `evaluate` TEXT NOT NULL,
                `total` INTEGER NOT NULL,
                `is_finish` INTEGER NOT NULL,
                `season_type` INTEGER NOT NULL,
                `path` TEXT NOT NULL,
                `created_at` TEXT NOT NULL DEFAULT (CURRENT_TIMESTAMP),
                `latest_row_at` TEXT NOT NULL,
                `rule` TEXT,
                `enabled` INTEGER NOT NULL DEFAULT 1
            )"
        )
        .await?;
        db.execute_unprepared(
            "INSERT INTO `bangumi_old` SELECT * FROM `bangumi`"
        )
        .await?;
        db.execute_unprepared("DROP TABLE `bangumi`").await?;
        db.execute_unprepared("ALTER TABLE `bangumi_old` RENAME TO `bangumi`").await?;
        Ok(())
    }
}
