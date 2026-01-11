use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        manager
            .create_table(
                Table::create()
                    .table(Bangumi::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Bangumi::Id)
                            .unsigned()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Bangumi::SeasonId).unique_key().unsigned().not_null())
                    .col(ColumnDef::new(Bangumi::MediaId).unsigned().not_null())
                    .col(ColumnDef::new(Bangumi::Title).string().not_null())
                    .col(ColumnDef::new(Bangumi::Cover).string().not_null())
                    .col(ColumnDef::new(Bangumi::Evaluate).string().not_null())
                    .col(ColumnDef::new(Bangumi::Total).small_unsigned().not_null())
                    .col(ColumnDef::new(Bangumi::IsFinish).boolean().not_null())
                    .col(ColumnDef::new(Bangumi::SeasonType).small_unsigned().not_null())
                    .col(ColumnDef::new(Bangumi::Path).string().not_null())
                    .col(
                        ColumnDef::new(Bangumi::CreatedAt)
                            .timestamp()
                            .default(Expr::current_timestamp())
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Bangumi::LatestRowAt)
                            .timestamp()
                            .not_null()
                            .default("1970-01-01 00:00:00"),
                    )
                    .col(ColumnDef::new(Bangumi::Rule).text().null())
                    .col(ColumnDef::new(Bangumi::Enabled).boolean().not_null().default(true))
                    .to_owned(),
            )
            .await?;
        manager
            .drop_index(Index::drop().table(Video::Table).name("idx_video_unique").to_owned())
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Video::Table)
                    .add_column(ColumnDef::new(Video::BangumiId).unsigned().null())
                    .to_owned(),
            )
            .await?;
        db.execute_unprepared("CREATE UNIQUE INDEX `idx_video_unique` ON `video` (ifnull(`collection_id`, -1), ifnull(`favorite_id`, -1), ifnull(`watch_later_id`, -1), ifnull(`submission_id`, -1), ifnull(`bangumi_id`, -1), `bvid`)")
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        manager
            .drop_index(Index::drop().table(Video::Table).name("idx_video_unique").to_owned())
            .await?;
        db.execute_unprepared("DELETE FROM video WHERE bangumi_id IS NOT NULL")
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Video::Table)
                    .drop_column(Video::BangumiId)
                    .to_owned(),
            )
            .await?;
        db.execute_unprepared("CREATE UNIQUE INDEX `idx_video_unique` ON `video` (ifnull(`collection_id`, -1), ifnull(`favorite_id`, -1), ifnull(`watch_later_id`, -1), ifnull(`submission_id`, -1), `bvid`)")
            .await?;
        manager.drop_table(Table::drop().table(Bangumi::Table).to_owned()).await
    }
}

#[derive(DeriveIden)]
enum Bangumi {
    Table,
    Id,
    SeasonId,
    MediaId,
    Title,
    Cover,
    Evaluate,
    Total,
    IsFinish,
    SeasonType,
    Path,
    CreatedAt,
    LatestRowAt,
    Rule,
    Enabled,
}

#[derive(DeriveIden)]
enum Video {
    Table,
    BangumiId,
}
