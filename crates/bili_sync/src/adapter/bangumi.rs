use std::borrow::Cow;
use std::path::Path;
use std::pin::Pin;

use anyhow::{Result, ensure};
use bili_sync_entity::rule::Rule;
use bili_sync_entity::*;
use futures::Stream;
use sea_orm::ActiveValue::Set;
use sea_orm::entity::prelude::*;
use sea_orm::sea_query::SimpleExpr;
use sea_orm::{DatabaseConnection, Unchanged};

use crate::adapter::{_ActiveModel, VideoSource, VideoSourceEnum};
use crate::bilibili::{BangumiList, BiliClient, Credential, VideoInfo};

impl VideoSource for bangumi::Model {
    fn display_name(&self) -> Cow<'static, str> {
        format!("番剧「{}」", self.title).into()
    }

    fn filter_expr(&self) -> SimpleExpr {
        video::Column::BangumiId.eq(self.id)
    }

    fn set_relation_id(&self, video_model: &mut video::ActiveModel) {
        video_model.bangumi_id = Set(Some(self.id));
    }

    fn path(&self) -> &Path {
        Path::new(self.path.as_str())
    }

    fn get_latest_row_at(&self) -> DateTime {
        self.latest_row_at
    }

    fn update_latest_row_at(&self, datetime: DateTime) -> _ActiveModel {
        _ActiveModel::Bangumi(bangumi::ActiveModel {
            id: Unchanged(self.id),
            latest_row_at: Set(datetime),
            ..Default::default()
        })
    }

    fn rule(&self) -> &Option<Rule> {
        &self.rule
    }

    async fn refresh<'a>(
        self,
        bili_client: &'a BiliClient,
        credential: &'a Credential,
        connection: &'a DatabaseConnection,
    ) -> Result<(
        VideoSourceEnum,
        Pin<Box<dyn Stream<Item = Result<VideoInfo>> + Send + 'a>>,
    )> {
        let mut bangumi = BangumiList::new(bili_client, self.season_id, credential);

        // 解析 selected_section_ids，只有在非空时才设置过滤
        if !self.selected_section_ids.is_empty()
            && let Ok(section_ids) = serde_json::from_str::<Vec<i64>>(&self.selected_section_ids)
            && !section_ids.is_empty()
        {
            bangumi = bangumi.with_selected_sections(section_ids);
        }

        let bangumi_info = bangumi.get_info().await?;
        ensure!(
            bangumi_info.season_id == self.season_id,
            "bangumi season_id mismatch: {} != {}",
            bangumi_info.season_id,
            self.season_id
        );
        let updated_model = bangumi::ActiveModel {
            id: Unchanged(self.id),
            title: Set(bangumi_info.title),
            ..Default::default()
        }
        .update(connection)
        .await?;
        Ok((updated_model.into(), Box::pin(bangumi.into_video_stream())))
    }

    async fn delete_from_db(self, conn: &impl ConnectionTrait) -> Result<()> {
        self.delete(conn).await?;
        Ok(())
    }
}
