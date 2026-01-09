use anyhow::{Context, Result, anyhow};
use async_stream::try_stream;
use chrono::{DateTime, Utc};
use futures::Stream;

use crate::bilibili::{BiliClient, Credential, Validate, VideoInfo};

pub struct BangumiList<'a> {
    client: &'a BiliClient,
    season_id: i64,
    credential: &'a Credential,
}

#[derive(Debug, serde::Deserialize)]
pub struct BangumiListInfo {
    pub season_id: i64,
    pub media_id: i64,
    pub title: String,
    pub cover: String,
    pub evaluate: String,
    pub total: u16,
    pub is_finish: bool,
    pub season_type: u16,
}

#[derive(Debug, serde::Deserialize)]
struct SeasonData {
    #[serde(default)]
    pub episodes: Vec<Episode>,
}

#[derive(Debug, serde::Deserialize)]
struct Episode {
    pub id: i64,              // ep_id
    pub aid: i64,             // 视频 aid
    pub bvid: String,
    pub cid: i64,
    pub title: String,        // 集标题
    pub long_title: String,   // 集副标题
    #[serde(default)]
    pub badge: String,
    #[serde(default)]
    pub section_type: i32,
    #[serde(default)]
    pub pub_time: i64,        // 发布时间戳
}

impl<'a> BangumiList<'a> {
    pub fn new(client: &'a BiliClient, season_id: i64, credential: &'a Credential) -> Self {
        Self {
            client,
            season_id,
            credential,
        }
    }

    pub async fn get_info(&self) -> Result<BangumiListInfo> {
        let res = self
            .client
            .request(
                reqwest::Method::GET,
                "https://api.bilibili.com/pgc/view/web/season",
                self.credential,
            )
            .await
            .query(&[("season_id", self.season_id)])
            .send()
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await?
            .validate()?;

        let data = &res["result"];

        // 处理 season_id 可能为 null 的情况
        let season_id = match data.get("season_id") {
            Some(v) if !v.is_null() => v
                .as_i64()
                .ok_or_else(|| anyhow!("invalid season_id type: expected i64"))?,
            None => {
                // 如果 API 没有返回 season_id，使用请求中的 season_id
                self.season_id
            }
            _ => return Err(anyhow!("season_id is null in API response")),
        };

        // 获取 media_id，如果为空则使用 season_id 作为默认值
        let media_id = data.get("media_id")
            .and_then(|v| if !v.is_null() { v.as_i64() } else { None })
            .unwrap_or(season_id);

        let title = data["title"]
            .as_str()
            .unwrap_or_default()
            .to_string();

        let cover = data["cover"]
            .as_str()
            .unwrap_or_default()
            .to_string();

        let evaluate = data["rating"]
            .as_object()
            .and_then(|r| r.get("score"))
            .and_then(|s| s.as_f64())
            .map(|s| s.to_string())
            .unwrap_or_default();

        let total = data["total"]
            .as_u64()
            .unwrap_or(0) as u16;

        let is_finish = data.get("is_finish")
            .and_then(|v| v.as_u64())
            .map(|v| v == 1)
            .unwrap_or(false);

        let season_type = data.get("type")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u16;

        Ok(BangumiListInfo {
            season_id,
            media_id,
            title,
            cover,
            evaluate,
            total,
            is_finish,
            season_type,
        })
    }

    async fn get_episodes(&self) -> Result<Vec<Episode>> {
        let mut res = self
            .client
            .request(
                reqwest::Method::GET,
                "https://api.bilibili.com/pgc/view/web/season",
                self.credential,
            )
            .await
            .query(&[("season_id", self.season_id)])
            .send()
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await?
            .validate()?;

        // 检查 result 是否为 null
        if res["result"].is_null() {
            return Err(anyhow!(
                "bangumi season_id {} is not available (possibly region-restricted or removed)",
                self.season_id
            ));
        }

        let season_data: SeasonData = serde_json::from_value(res["result"].take())
            .with_context(|| format!("failed to parse bangumi episodes for season_id {}", self.season_id))?;
        Ok(season_data.episodes)
    }

    pub fn into_video_stream(self) -> impl Stream<Item = Result<VideoInfo>> + 'a {
        try_stream! {
            let episodes = self.get_episodes().await
                .with_context(|| format!("failed to get episodes of bangumi season_id {}", self.season_id))?;

            if episodes.is_empty() {
                Err(anyhow!("no episodes found in bangumi season_id {}", self.season_id))?;
            }

            // 获取番剧信息，包含番剧标题
            let bangumi_info = self.get_info().await
                .with_context(|| format!("failed to get bangumi info for season_id {}", self.season_id))?;

            for episode in episodes {
                // 跳过 PV、预告等非正片内容（section_type: 1 表示预告片）
                if episode.section_type == 1 {
                    continue;
                }

                let pubtime = DateTime::from_timestamp(episode.pub_time, 0).unwrap_or_else(Utc::now);
                tracing::debug!(
                    "番剧剧集: bvid={}, title={}, pub_time={}, pubtime={}",
                    episode.bvid,
                    episode.title,
                    episode.pub_time,
                    pubtime
                );

                // 解析集数
                let episode_number = episode.title.parse::<i32>().ok();

                // 构建完整的标题：番剧名称 + 集数信息
                // 例如：灵笼 第一季_第001话
                let full_title = format!("{}_{}", bangumi_info.title, episode.title);

                let video_info = VideoInfo::Bangumi {
                    title: full_title,
                    season_id: self.season_id.to_string(),
                    ep_id: episode.id.to_string(),
                    bvid: episode.bvid.clone(),
                    cid: episode.cid.to_string(),
                    aid: episode.aid.to_string(),
                    cover: bangumi_info.cover.clone(),
                    intro: String::new(),
                    pubtime,
                    show_title: Some(episode.title.clone()),
                    season_number: None,
                    episode_number,
                    share_copy: None,
                    show_season_type: Some(bangumi_info.season_type as i32),
                    actors: None,
                };
                yield video_info;
            }
        }
    }
}
