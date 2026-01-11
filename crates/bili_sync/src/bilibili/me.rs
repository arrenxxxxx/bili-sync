use anyhow::{Result, ensure};
use reqwest::Method;
use serde::Deserialize;

use crate::bilibili::{BiliClient, Credential, Validate};

pub struct Me<'a> {
    client: &'a BiliClient,
    credential: &'a Credential,
}

impl<'a> Me<'a> {
    pub fn new(client: &'a BiliClient, credential: &'a Credential) -> Self {
        Self { client, credential }
    }

    pub async fn get_created_favorites(&self) -> Result<Option<Vec<FavoriteItem>>> {
        ensure!(
            !self.mid().is_empty(),
            "未获取到用户 ID，请确保填写设置中的 B 站认证信息"
        );
        let mut resp = self
            .client
            .request(
                Method::GET,
                "https://api.bilibili.com/x/v3/fav/folder/created/list-all",
                self.credential,
            )
            .await
            .query(&[("up_mid", &self.mid())])
            .send()
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await?
            .validate()?;
        Ok(serde_json::from_value(resp["data"]["list"].take())?)
    }

    pub async fn get_followed_collections(&self, page_num: i32, page_size: i32) -> Result<Collections> {
        ensure!(
            !self.mid().is_empty(),
            "未获取到用户 ID，请确保填写设置中的 B 站认证信息"
        );
        let mut resp = self
            .client
            .request(
                Method::GET,
                "https://api.bilibili.com/x/v3/fav/folder/collected/list",
                self.credential,
            )
            .await
            .query(&[("up_mid", self.mid()), ("platform", "web")])
            .query(&[("pn", page_num), ("ps", page_size)])
            .send()
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await?
            .validate()?;
        Ok(serde_json::from_value(resp["data"].take())?)
    }

    pub async fn get_followed_uppers(
        &self,
        page_num: i32,
        page_size: i32,
        name: Option<&str>,
    ) -> Result<FollowedUppers> {
        ensure!(
            !self.mid().is_empty(),
            "未获取到用户 ID，请确保填写设置中的 B 站认证信息"
        );
        let url = if name.is_some() {
            "https://api.bilibili.com/x/relation/followings/search"
        } else {
            "https://api.bilibili.com/x/relation/followings"
        };
        let mut request = self
            .client
            .request(Method::GET, url, self.credential)
            .await
            .query(&[("vmid", self.mid())])
            .query(&[("pn", page_num), ("ps", page_size)]);
        if let Some(name) = name {
            request = request.query(&[("name", name)]);
        }
        let mut resp = request
            .send()
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await?
            .validate()?;
        Ok(serde_json::from_value(resp["data"].take())?)
    }

    pub async fn get_followed_bangumi(
        &self,
        page_num: i32,
        page_size: i32,
        follow_type: BangumiType,
    ) -> Result<FollowedBangumi> {
        ensure!(
            !self.mid().is_empty(),
            "未获取到用户 ID，请确保填写设置中的 B 站认证信息"
        );
        let mut resp = self
            .client
            .request(
                Method::GET,
                "https://api.bilibili.com/x/space/bangumi/follow/list",
                self.credential,
            )
            .await
            .query(&[("vmid", self.mid())])
            .query(&[("pn", page_num), ("ps", page_size)])
            .query(&[("type", follow_type as i32)])
            .query(&[("platform", "web")])
            .query(&[("follow_status", 0)])
            .send()
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await?
            .validate()?;
        Ok(serde_json::from_value(resp["data"].take())?)
    }

    fn mid(&self) -> &str {
        &self.credential.dedeuserid
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct FavoriteItem {
    pub title: String,
    pub media_count: i64,
    pub id: i64,
    pub mid: i64,
}

#[derive(Debug, serde::Deserialize)]
pub struct CollectionItem {
    pub id: i64,
    pub fid: i64,
    pub mid: i64,
    pub state: i32,
    pub title: String,
    pub media_count: i64,
}

#[derive(Debug, serde::Deserialize)]
pub struct Collections {
    pub count: i64,
    pub list: Option<Vec<CollectionItem>>,
}

#[derive(Debug, serde::Deserialize)]
pub struct FollowedUppers {
    pub total: i64,
    pub list: Vec<FollowedUpper>,
}

#[derive(Debug, serde::Deserialize)]
pub struct FollowedUpper {
    pub mid: i64,
    pub uname: String,
    pub face: String,
    pub sign: String,
}

#[derive(Debug, Clone, Copy)]
pub enum BangumiType {
    Anime = 1, // 番剧
    Drama = 2, // 追剧
}

#[derive(Debug, serde::Deserialize)]
pub struct FollowedBangumi {
    #[serde(alias = "count", default = "default_total")]
    pub total: i64,
    #[serde(default)]
    pub list: Vec<BangumiItem>,
}

fn default_total() -> i64 {
    0
}

#[derive(Debug, serde::Deserialize)]
pub struct BangumiItem {
    pub season_id: i64,
    #[allow(dead_code)]
    pub media_id: i64,
    pub title: String,
    pub cover: String,
    #[serde(default)]
    pub evaluate: String,
    /// 总集数，优先使用 formal_ep_count（正式集数），如果不存在则使用 total/total_count
    #[serde(default = "default_total_count")]
    pub total_count: i32,
    #[serde(default, deserialize_with = "deserialize_bool_from_int")]
    pub is_finish: bool,
    pub season_type: u16,
    /// 正式集数（对于连载番剧更准确）
    #[serde(default)]
    pub formal_ep_count: i32,
}

fn default_total_count() -> i32 {
    0
}

/// 将整数反序列化为布尔值：0 -> false, 1 -> true
fn deserialize_bool_from_int<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match Deserialize::deserialize(deserializer)? {
        0 => Ok(false),
        1 => Ok(true),
        other => Err(serde::de::Error::invalid_value(
            serde::de::Unexpected::Signed(other as i64),
            &"0 or 1",
        )),
    }
}
