use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(VideoSource::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(VideoSource::Id)
                            .unsigned()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(VideoSource::Name).string().not_null())
                    .col(ColumnDef::new(VideoSource::Path).string().not_null())
                    .col(ColumnDef::new(VideoSource::Type).integer().not_null()) // 1=番剧
                    .col(ColumnDef::new(VideoSource::LatestRowAt).string().not_null())
                    .col(ColumnDef::new(VideoSource::CreatedAt).string().not_null())
                    // 番剧相关字段
                    .col(ColumnDef::new(VideoSource::SeasonId).string().null())
                    .col(ColumnDef::new(VideoSource::MediaId).string().null())
                    .col(ColumnDef::new(VideoSource::EpId).string().null())
                    .col(ColumnDef::new(VideoSource::DownloadAllSeasons).boolean().null())
                    .col(ColumnDef::new(VideoSource::PageNameTemplate).string().null())
                    .col(ColumnDef::new(VideoSource::SelectedSeasons).string().null()) // JSON array
                    // 通用配置
                    .col(ColumnDef::new(VideoSource::Enabled).boolean().not_null().default(true))
                    .col(ColumnDef::new(VideoSource::ScanDeletedVideos).boolean().not_null().default(false))
                    .col(ColumnDef::new(VideoSource::CachedEpisodes).string().null()) // JSON cache
                    .col(ColumnDef::new(VideoSource::CacheUpdatedAt).string().null())
                    .col(ColumnDef::new(VideoSource::KeywordFilters).string().null())
                    .col(ColumnDef::new(VideoSource::KeywordFilterMode).string().null())
                    .col(ColumnDef::new(VideoSource::BlacklistKeywords).string().null())
                    .col(ColumnDef::new(VideoSource::WhitelistKeywords).string().null())
                    .col(ColumnDef::new(VideoSource::KeywordCaseSensitive).boolean().not_null().default(false))
                    .col(ColumnDef::new(VideoSource::AudioOnly).boolean().not_null().default(false))
                    .col(ColumnDef::new(VideoSource::AudioOnlyM4aOnly).boolean().not_null().default(false))
                    .col(ColumnDef::new(VideoSource::FlatFolder).boolean().not_null().default(false))
                    .col(ColumnDef::new(VideoSource::DownloadDanmaku).boolean().not_null().default(true))
                    .col(ColumnDef::new(VideoSource::DownloadSubtitle).boolean().not_null().default(true))
                    .col(ColumnDef::new(VideoSource::AiRename).boolean().not_null().default(false))
                    .col(ColumnDef::new(VideoSource::AiRenameVideoPrompt).string().not_null().default(""))
                    .col(ColumnDef::new(VideoSource::AiRenameAudioPrompt).string().not_null().default(""))
                    .col(ColumnDef::new(VideoSource::AiRenameEnableMultiPage).boolean().not_null().default(false))
                    .col(ColumnDef::new(VideoSource::AiRenameEnableCollection).boolean().not_null().default(false))
                    .col(ColumnDef::new(VideoSource::AiRenameEnableBangumi).boolean().not_null().default(false))
                    .to_owned(),
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(VideoSource::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum VideoSource {
    Table,
    Id,
    Name,
    Path,
    Type,
    LatestRowAt,
    CreatedAt,
    SeasonId,
    MediaId,
    EpId,
    DownloadAllSeasons,
    PageNameTemplate,
    SelectedSeasons,
    Enabled,
    ScanDeletedVideos,
    CachedEpisodes,
    CacheUpdatedAt,
    KeywordFilters,
    KeywordFilterMode,
    BlacklistKeywords,
    WhitelistKeywords,
    KeywordCaseSensitive,
    AudioOnly,
    AudioOnlyM4aOnly,
    FlatFolder,
    DownloadDanmaku,
    DownloadSubtitle,
    AiRename,
    AiRenameVideoPrompt,
    AiRenameAudioPrompt,
    AiRenameEnableMultiPage,
    AiRenameEnableCollection,
    AiRenameEnableBangumi,
}
