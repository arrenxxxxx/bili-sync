use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // 添加 source_id 和 source_type 字段到 video 表
        manager
            .alter_table(
                Table::alter()
                    .table(Video::Table)
                    .add_column(ColumnDef::new(Video::SourceId).unsigned().null())
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Video::Table)
                    .add_column(ColumnDef::new(Video::SourceType).integer().null())
                    .to_owned(),
            )
            .await?;

        // 删除旧的唯一索引
        manager
            .drop_index(Index::drop().table(Video::Table).name("idx_video_unique").to_owned())
            .await?;

        // 创建新的统一唯一索引（包含 source_id 和 source_type）
        db.execute_unprepared(
            "CREATE UNIQUE INDEX `idx_video_unique` ON `video` (ifnull(`collection_id`, -1), ifnull(`favorite_id`, -1), ifnull(`watch_later_id`, -1), ifnull(`submission_id`, -1), ifnull(`bangumi_id`, -1), ifnull(`source_id`, -1), `source_type`, `bvid`)"
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // 删除新的统一唯一索引
        manager
            .drop_index(Index::drop().table(Video::Table).name("idx_video_unique").to_owned())
            .await?;

        // 删除 source_type 和 source_id 字段
        manager
            .alter_table(
                Table::alter()
                    .table(Video::Table)
                    .drop_column(Video::SourceType)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Video::Table)
                    .drop_column(Video::SourceId)
                    .to_owned(),
            )
            .await?;

        // 恢复旧的唯一索引
        db.execute_unprepared(
            "CREATE UNIQUE INDEX `idx_video_unique` ON `video` (ifnull(`collection_id`, -1), ifnull(`favorite_id`, -1), ifnull(`watch_later_id`, -1), ifnull(`submission_id`, -1), ifnull(`bangumi_id`, -1), `bvid`)"
        )
        .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum Video {
    Table,
    SourceId,
    SourceType,
}
