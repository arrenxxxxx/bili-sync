use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Video::Table)
                    .add_column(string_null(Video::SectionTitle))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Video::Table)
                    .drop_column(Video::SectionTitle)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Video {
    Table,
    SectionTitle,
}
