use anyhow::{Context, Result, anyhow};
use async_stream::try_stream;
use chrono::{DateTime, Utc};
use futures::Stream;
use serde::{Deserialize, Serialize};

use crate::bilibili::{BiliClient, Credential, Validate, VideoInfo};

/// 剧集与其所属 section 的配对
#[derive(Clone, Debug)]
struct EpisodeWithSection {
    episode: Episode,
    section_title: Option<String>,
}

pub struct BangumiList<'a> {
    client: &'a BiliClient,
    season_id: i64,
    credential: &'a Credential,
    /// 可选：仅获取指定 section_id 的剧集
    selected_section_ids: Option<Vec<i64>>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectionInfo {
    pub id: i64,
    pub title: String,
    #[serde(rename = "type")]
    pub section_type: i32,
    pub episode_count: usize,
}

#[derive(Debug, serde::Deserialize)]
struct SeasonData {
    #[serde(default)]
    pub episodes: Vec<Episode>,
    #[serde(default)]
    #[serde(rename = "section")]
    pub sections: Vec<SectionData>,
    // 忽略其他字段
}

#[derive(Debug, serde::Deserialize)]
struct SectionData {
    pub id: i64,
    pub title: String,
    #[serde(rename = "type")]
    pub section_type: i32,
    #[serde(default)]
    pub episodes: Vec<Episode>,
    // 忽略其他字段
    #[serde(default)]
    pub attr: i32,
    #[serde(default)]
    pub episode_id: i64,
    #[serde(default)]
    pub episode_ids: Vec<i64>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct Episode {
    #[serde(default)]
    pub id: i64, // ep_id
    #[serde(default)]
    pub aid: i64, // 视频 aid
    #[serde(default)]
    pub bvid: String,
    #[serde(default)]
    pub cid: i64,
    #[serde(default)]
    pub title: String, // 集标题
    #[serde(default)]
    pub long_title: String, // 集副标题
    #[serde(default)]
    pub badge: String,
    #[serde(default)]
    pub section_type: i32,
    #[serde(default)]
    pub pub_time: i64, // 发布时间戳
    // 忽略其他复杂字段
    #[serde(default)]
    pub cover: String,
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub link: String,
}

impl<'a> BangumiList<'a> {
    pub fn new(client: &'a BiliClient, season_id: i64, credential: &'a Credential) -> Self {
        Self {
            client,
            season_id,
            credential,
            selected_section_ids: None,
        }
    }

    /// 设置要下载的 section_id 列表
    pub fn with_selected_sections(mut self, section_ids: Vec<i64>) -> Self {
        self.selected_section_ids = Some(section_ids);
        self
    }

    /// 获取可用的 section 列表
    pub async fn get_sections(&self) -> Result<Vec<SectionInfo>> {
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

        let sections: Vec<SectionData> = serde_json::from_value(res["result"]["section"].clone())
            .with_context(|| format!("failed to parse sections for season_id {}", self.season_id))?;

        Ok(sections
            .into_iter()
            .map(|s| SectionInfo {
                id: s.id,
                title: s.title,
                section_type: s.section_type,
                episode_count: s.episodes.len(),
            })
            .collect())
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
        let media_id = data
            .get("media_id")
            .and_then(|v| if !v.is_null() { v.as_i64() } else { None })
            .unwrap_or(season_id);

        let title = data["title"].as_str().unwrap_or_default().to_string();

        let cover = data["cover"].as_str().unwrap_or_default().to_string();

        let evaluate = data["rating"]
            .as_object()
            .and_then(|r| r.get("score"))
            .and_then(|s| s.as_f64())
            .map(|s| s.to_string())
            .unwrap_or_default();

        let total = data["total"].as_u64().unwrap_or(0) as u16;

        let is_finish = data
            .get("is_finish")
            .and_then(|v| v.as_u64())
            .map(|v| v == 1)
            .unwrap_or(false);

        let season_type = data.get("type").and_then(|v| v.as_u64()).unwrap_or(0) as u16;

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

    async fn get_episodes(&self) -> Result<Vec<EpisodeWithSection>> {
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

        tracing::debug!(
            "bangumi season_id {}: {} episodes in main list, {} sections parsed",
            self.season_id,
            season_data.episodes.len(),
            season_data.sections.len()
        );

        // 如果指定了 selected_section_ids，获取正片 + 选中的花絮 section
        if let Some(ref section_ids) = self.selected_section_ids {
            let mut episodes_with_section = Vec::new();

            // 1. 先添加所有正片（section_type == 0）
            for episode in &season_data.episodes {
                if episode.section_type == 0 {
                    episodes_with_section.push(EpisodeWithSection {
                        episode: episode.clone(),
                        section_title: None,
                    });
                }
            }

            // 2. 再添加选中的花絮 section 中的剧集
            for section in &season_data.sections {
                if section_ids.contains(&section.id) {
                    let section_title = section.title.clone();
                    for episode in &section.episodes {
                        episodes_with_section.push(EpisodeWithSection {
                            episode: episode.clone(),
                            section_title: Some(section_title.clone()),
                        });
                    }
                }
            }

            tracing::info!(
                "bangumi season_id {}: got {} main episodes + {} extra episodes from selected sections",
                self.season_id,
                season_data.episodes.iter().filter(|e| e.section_type == 0).count(),
                episodes_with_section.len() - season_data.episodes.iter().filter(|e| e.section_type == 0).count()
            );
            Ok(episodes_with_section)
        } else {
            // 如果没有指定，获取所有正片（section_type == 0）
            let episodes: Vec<_> = season_data
                .episodes
                .into_iter()
                .filter(|e| e.section_type == 0)
                .map(|e| EpisodeWithSection {
                    episode: e,
                    section_title: None,
                })
                .collect();
            tracing::debug!(
                "bangumi season_id {}: got {} main episodes (section_type == 0)",
                self.season_id,
                episodes.len()
            );
            Ok(episodes)
        }
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

            for EpisodeWithSection { episode, section_title } in episodes {
                let pubtime = DateTime::from_timestamp(episode.pub_time, 0).unwrap_or_else(Utc::now);

                // 解析集数
                let episode_number = episode.title.parse::<i32>().ok();

                // 构建完整的标题：番剧名称 + 集数信息
                // 优先使用 long_title，如果为空则使用 title
                // 例如：灵笼 第一季_第001话
                let episode_title = if !episode.long_title.is_empty() {
                    &episode.long_title
                } else if !episode.title.is_empty() && !episode.title.contains('.') {
                    // title 不包含 '.' 说明可能是正常标题，而不是文件名
                    &episode.title
                } else {
                    // 使用 badge 作为标题（如 "PV"、"预告" 等）
                    if !episode.badge.is_empty() {
                        &episode.badge
                    } else {
                        &episode.title
                    }
                };
                let full_title = format!("{}_{}", bangumi_info.title, episode_title);

                // 对于花絮（有 section_title 的情况），show_title 使用简单的 episode_title
                // 而不是完整的 episode.title，避免文件名过长
                let show_title = if section_title.is_some() {
                    episode_title.to_string()
                } else {
                    episode.title.clone()
                };

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
                    show_title: Some(show_title),
                    section_title,
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
