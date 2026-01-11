use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result, anyhow, bail};
use bili_sync_entity::*;
use futures::stream::FuturesUnordered;
use futures::{Stream, StreamExt, TryStreamExt};
use sea_orm::ActiveValue::Set;
use sea_orm::TransactionTrait;
use sea_orm::entity::prelude::*;
use tokio::fs;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::adapter::{VideoSource, VideoSourceEnum};
use crate::bilibili::{BestStream, BiliClient, BiliError, Dimension, PageInfo, Video, VideoInfo};
use crate::config::{ARGS, Config, PathSafeTemplate};
use crate::downloader::Downloader;
use crate::error::ExecutionStatus;
use crate::utils::download_context::DownloadContext;
use crate::utils::format_arg::{bangumi_page_format_args, page_format_args, video_format_args};
use crate::utils::model::{
    create_pages, create_videos, filter_unfilled_videos, filter_unhandled_video_pages, update_pages_model,
    update_videos_model,
};
use crate::utils::nfo::{NFO, ToNFO};
use crate::utils::rule::FieldEvaluatable;
use crate::utils::status::{PageStatus, STATUS_OK, VideoStatus};

// 全局番剧季度标题缓存
fn season_title_cache() -> &'static Arc<Mutex<HashMap<String, String>>> {
    static CACHE: OnceLock<Arc<Mutex<HashMap<String, String>>>> = OnceLock::new();
    CACHE.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

/// 获取番剧季标题，优先从缓存获取，缓存未命中时从API获取
async fn get_cached_season_title(
    bili_client: &BiliClient,
    season_id: &str,
    _token: CancellationToken,
) -> Option<String> {
    // 先检查缓存
    if let Ok(cache) = season_title_cache().lock() {
        if let Some(title) = cache.get(season_id) {
            return Some(title.clone());
        }
    }

    // 缓存未命中，从API获取
    get_season_title_from_api(bili_client, season_id).await
}

async fn get_season_title_from_api(bili_client: &BiliClient, season_id: &str) -> Option<String> {
    let url = format!("https://api.bilibili.com/pgc/view/web/season?season_id={}", season_id);
    tracing::debug!("获取番剧标题: {}", url);

    let res = bili_client
        .client
        .request(
            reqwest::Method::GET,
            &url,
            None, // 番剧API不需要credential
        )
        .send()
        .await
        .ok()?;

    if !res.status().is_success() {
        tracing::warn!("获取番剧标题失败，状态码: {}", res.status());
        return None;
    }

    let json = res.json::<serde_json::Value>().await.ok()?;
    if json["code"].as_i64()? != 0 {
        tracing::warn!("获取番剧标题失败，API错误: {}", json["message"]);
        return None;
    }

    let title = json["result"]["title"].as_str()?;
    let normalized_title = title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" （", "（")
        .replace(" (", "(");

    tracing::debug!("获取到番剧标题: {} (season_id={})", normalized_title, season_id);

    // 缓存标题
    if let Ok(mut cache) = season_title_cache().lock() {
        cache.insert(season_id.to_string(), normalized_title.clone());
    }

    Some(normalized_title)
}

/// 完整地处理某个视频来源
pub async fn process_video_source(
    video_source: VideoSourceEnum,
    bili_client: &BiliClient,
    connection: &DatabaseConnection,
    template: &handlebars::Handlebars<'_>,
    config: &Config,
) -> Result<()> {
    // 预创建视频源目录，提前检测目录是否可写
    video_source.create_dir_all().await?;
    // 从参数中获取视频列表的 Model 与视频流
    let (video_source, video_streams) = video_source
        .refresh(bili_client, &config.credential, connection)
        .await?;
    // 从视频流中获取新视频的简要信息，写入数据库
    refresh_video_source(&video_source, video_streams, connection).await?;
    // 单独请求视频详情接口，获取视频的详情信息与所有的分页，写入数据库
    fetch_video_details(bili_client, &video_source, connection, config).await?;
    if ARGS.scan_only {
        warn!("已开启仅扫描模式，跳过视频下载..");
    } else {
        // 从数据库中查找所有未下载的视频与分页，下载并处理
        download_unprocessed_videos(bili_client, &video_source, connection, template, config).await?;
    }
    Ok(())
}

/// 请求接口，获取视频列表中所有新添加的视频信息，将其写入数据库
pub async fn refresh_video_source<'a>(
    video_source: &VideoSourceEnum,
    video_streams: Pin<Box<dyn Stream<Item = Result<VideoInfo>> + 'a + Send>>,
    connection: &DatabaseConnection,
) -> Result<()> {
    video_source.log_refresh_video_start();
    let latest_row_at = video_source.get_latest_row_at().and_utc();
    let mut max_datetime = latest_row_at;
    let mut error = Ok(());
    let mut video_streams = video_streams
        .enumerate()
        .take_while(|(idx, res)| {
            match res {
                Err(e) => {
                    // 这里拿到的 e 是引用，无法直接传递所有权
                    // 对于 BiliError，我们需要克隆内部的错误并附带原来的上下文，方便外部检查错误类型
                    // 对于其他错误只保留字符串信息用作提示
                    if let Some(inner) = e.downcast_ref::<BiliError>() {
                        error = Err(inner.clone()).context(e.to_string());
                    } else {
                        error = Err(anyhow!("{:#}", e));
                    }
                    futures::future::ready(false)
                }
                Ok(v) => {
                    // 虽然 video_streams 是从新到旧的，但由于此处是分页请求，极端情况下可能发生访问完第一页时插入了两整页视频的情况
                    // 此时获取到的第二页视频比第一页的还要新，因此为了确保正确，理应对每一页的第一个视频进行时间比较
                    // 但在 streams 的抽象下，无法判断具体是在哪里分页的，所以暂且对每个视频都进行比较，应该不会有太大性能损失
                    let release_datetime = v.release_datetime();
                    if release_datetime > &max_datetime {
                        max_datetime = *release_datetime;
                    }
                    futures::future::ready(video_source.should_take(*idx, release_datetime, &latest_row_at))
                }
            }
        })
        .filter_map(|(idx, res)| futures::future::ready(video_source.should_filter(idx, res, &latest_row_at)))
        .chunks(10);
    let mut count = 0;
    while let Some(videos_info) = video_streams.next().await {
        count += videos_info.len();
        create_videos(videos_info, video_source, connection).await?;
    }
    // 如果获取视频分页过程中发生了错误，直接在此处返回，不更新 latest_row_at
    error?;
    if max_datetime != latest_row_at {
        video_source
            .update_latest_row_at(max_datetime.naive_utc())
            .save(connection)
            .await?;
    }
    video_source.log_refresh_video_end(count);
    Ok(())
}

/// 筛选出所有未获取到全部信息的视频，尝试补充其详细信息
pub async fn fetch_video_details(
    bili_client: &BiliClient,
    video_source: &VideoSourceEnum,
    connection: &DatabaseConnection,
    config: &Config,
) -> Result<()> {
    video_source.log_fetch_video_start();
    let videos_model = filter_unfilled_videos(video_source.filter_expr(), connection).await?;
    let semaphore = Semaphore::new(config.concurrent_limit.video);
    let semaphore_ref = &semaphore;
    let tasks = videos_model
        .into_iter()
        .map(|video_model| async move {
            let _permit = semaphore_ref.acquire().await.context("acquire semaphore failed")?;
            let video = Video::new(bili_client, video_model.bvid.clone(), &config.credential);
            let info: Result<_> = async { Ok((video.get_tags().await?, video.get_view_info().await?)) }.await;
            match info {
                Err(e) => {
                    error!(
                        "获取视频 {} - {} 的详细信息失败，错误为：{:#}",
                        &video_model.bvid, &video_model.name, e
                    );
                    if let Some(BiliError::ErrorResponse(-404, _, _)) = e.downcast_ref::<BiliError>() {
                        let mut video_active_model: bili_sync_entity::video::ActiveModel = video_model.into();
                        video_active_model.valid = Set(false);
                        video_active_model.save(connection).await?;
                    }
                }
                Ok((tags, mut view_info)) => {
                    let VideoInfo::Detail { pages, .. } = &mut view_info else {
                        unreachable!()
                    };
                    // 构造 page model
                    let pages = std::mem::take(pages);
                    // 对于番剧花絮，使用友好的 page name（show_title）而不是原始文件名
                    let is_bangumi_extra = video_model
                        .section_title
                        .as_ref()
                        .map(|s| !s.is_empty())
                        .unwrap_or(false)
                        || video_model.episode_number.is_none()
                        || video_model.episode_number == Some(0);
                    let pages = pages
                        .into_iter()
                        .map(|mut p| {
                            // 对于番剧花絮，使用 show_title 作为 page name
                            if is_bangumi_extra {
                                if let Some(ref show_title) = video_model.show_title {
                                    p.name = show_title.clone();
                                }
                            }
                            p.into_active_model(video_model.id)
                        })
                        .collect::<Vec<page::ActiveModel>>();
                    // 更新 video model 的各项有关属性
                    // 花絮/PV/预告（没有集数）强制使用单页命名方式
                    let is_bangumi_extra =
                        video_model.episode_number.is_none() || video_model.episode_number == Some(0);
                    let mut video_active_model = view_info.into_detail_model(video_model);
                    video_source.set_relation_id(&mut video_active_model);
                    video_active_model.single_page = Set(Some(pages.len() == 1 || is_bangumi_extra));
                    video_active_model.tags = Set(Some(tags.into()));
                    video_active_model.should_download = Set(video_source.rule().evaluate(&video_active_model, &pages));
                    let txn = connection.begin().await?;
                    create_pages(pages, &txn).await?;
                    video_active_model.save(&txn).await?;
                    txn.commit().await?;
                }
            };
            Ok::<_, anyhow::Error>(())
        })
        .collect::<FuturesUnordered<_>>();
    tasks.try_collect::<Vec<_>>().await?;
    video_source.log_fetch_video_end();
    Ok(())
}

/// 下载所有未处理成功的视频
pub async fn download_unprocessed_videos(
    bili_client: &BiliClient,
    video_source: &VideoSourceEnum,
    connection: &DatabaseConnection,
    template: &handlebars::Handlebars<'_>,
    config: &Config,
) -> Result<()> {
    video_source.log_download_video_start();
    let semaphore = Semaphore::new(config.concurrent_limit.video);
    let downloader = Downloader::new(bili_client.client.clone());
    let cx = DownloadContext::new(bili_client, video_source, template, connection, &downloader, config);
    let unhandled_videos_pages = filter_unhandled_video_pages(video_source.filter_expr(), connection).await?;
    let mut assigned_upper = HashSet::new();
    let tasks = unhandled_videos_pages
        .into_iter()
        .map(|(video_model, pages_model)| {
            let should_download_upper = !assigned_upper.contains(&video_model.upper_id);
            assigned_upper.insert(video_model.upper_id);
            download_video_pages(video_model, pages_model, &semaphore, should_download_upper, cx)
        })
        .collect::<FuturesUnordered<_>>();
    let mut risk_control_related_error = None;
    let mut stream = tasks
        // 触发风控时设置 download_aborted 标记并终止流
        .take_while(|res| {
            if let Err(e) = res {
                if let Some(e) = e.downcast_ref::<BiliError>() {
                    if e.is_risk_control_related() {
                        risk_control_related_error = Some(e.clone());
                    }
                }
                // 记录错误日志
                tracing::error!("下载视频失败: {}", e);
            }
            futures::future::ready(risk_control_related_error.is_none())
        })
        // 过滤掉没有触发风控的普通 Err，只保留正确返回的 Model
        .filter_map(|res| {
            if res.is_err() {
                tracing::warn!("跳过下载失败的视频（非风控错误）");
            }
            futures::future::ready(res.ok())
        })
        // 将成功返回的 Model 按十个一组合并
        .chunks(10);
    while let Some(models) = stream.next().await {
        update_videos_model(models, connection).await?;
    }
    if let Some(e) = risk_control_related_error {
        bail!(e);
    }
    video_source.log_download_video_end();
    Ok(())
}

pub async fn download_video_pages(
    video_model: video::Model,
    page_models: Vec<page::Model>,
    semaphore: &Semaphore,
    should_download_upper: bool,
    cx: DownloadContext<'_>,
) -> Result<video::ActiveModel> {
    let _permit = semaphore.acquire().await.context("acquire semaphore failed")?;
    let mut status = VideoStatus::from(video_model.download_status);
    let separate_status = status.should_run();

    // 判断是否为番剧
    let is_bangumi =
        video_model.bangumi_id.is_some() || (video_model.source_id.is_some() && video_model.source_type == Some(1));

    // 未记录路径时填充，已经填充过路径时使用现有的
    let base_path = if !video_model.path.is_empty() {
        PathBuf::from(&video_model.path)
    } else {
        // 对于番剧花絮（有非空 section_title），直接使用视频源路径
        // 不使用 bangumi 模板，避免在 video.path 中存储剧集名目录
        if is_bangumi
            && video_model
                .section_title
                .as_ref()
                .map(|s| !s.is_empty())
                .unwrap_or(false)
        {
            cx.video_source.path().to_path_buf()
        } else {
            // 根据视频类型选择不同的模板
            let (template_name, format_args) = if is_bangumi {
                // 获取番剧标题（从缓存或API）
                let api_title = if let Some(ref season_id) = video_model.season_id {
                    get_cached_season_title(cx.bili_client, season_id, CancellationToken::new()).await
                } else {
                    None
                };

                (
                    "bangumi",
                    bangumi_page_format_args(
                        &video_model,
                        &page_models.first().unwrap(),
                        &cx.config.time_format,
                        api_title.as_deref(),
                    ),
                )
            } else {
                ("video", video_format_args(&video_model, &cx.config.time_format))
            };

            cx.video_source
                .path()
                .join(cx.template.path_safe_render(template_name, &format_args)?)
        }
    };
    let upper_id = video_model.upper_id.to_string();
    let base_upper_path = cx
        .config
        .upper_path
        .join(upper_id.chars().next().context("upper_id is empty")?.to_string())
        .join(upper_id);
    let is_single_page = video_model.single_page.context("single_page is null")?;
    // 对于单页视频，page 的下载已经足够
    // 对于多页视频，page 下载仅包含了分集内容，需要额外补上视频的 poster 的 tvshow.nfo
    let (res_1, res_2, res_3, res_4, res_5) = tokio::join!(
        // 下载视频封面
        fetch_video_poster(
            separate_status[0] && !is_single_page && !cx.config.skip_option.no_poster,
            &video_model,
            base_path.join("poster.jpg"),
            base_path.join("fanart.jpg"),
            cx
        ),
        // 生成视频信息的 nfo
        generate_video_nfo(
            separate_status[1] && !is_single_page && !cx.config.skip_option.no_video_nfo,
            &video_model,
            base_path.join("tvshow.nfo"),
            cx
        ),
        // 下载 Up 主头像
        fetch_upper_face(
            separate_status[2] && should_download_upper && !cx.config.skip_option.no_upper,
            &video_model,
            base_upper_path.join("folder.jpg"),
            cx
        ),
        // 生成 Up 主信息的 nfo
        generate_upper_nfo(
            separate_status[3] && should_download_upper && !cx.config.skip_option.no_upper,
            &video_model,
            base_upper_path.join("person.nfo"),
            cx,
        ),
        // 分发并执行分页下载的任务
        dispatch_download_page(separate_status[4], &video_model, page_models, &base_path, cx)
    );
    let results = [res_1.into(), res_2.into(), res_3.into(), res_4.into(), res_5.into()];
    status.update_status(&results);
    results
        .iter()
        .take(4)
        .zip(["封面", "详情", "作者头像", "作者详情"])
        .for_each(|(res, task_name)| match res {
            ExecutionStatus::Skipped => info!("处理视频「{}」{}已成功过，跳过", &video_model.name, task_name),
            ExecutionStatus::Succeeded => info!("处理视频「{}」{}成功", &video_model.name, task_name),
            ExecutionStatus::Ignored(e) => {
                error!(
                    "处理视频「{}」{}出现常见错误，已忽略：{:#}",
                    &video_model.name, task_name, e
                )
            }
            ExecutionStatus::Failed(e) => {
                error!("处理视频「{}」{}失败：{:#}", &video_model.name, task_name, e)
            }
            ExecutionStatus::Fixed(_) => unreachable!(),
        });
    for result in results {
        if let ExecutionStatus::Failed(e) = result {
            if let Ok(e) = e.downcast::<BiliError>() {
                if e.is_risk_control_related() {
                    bail!(e);
                }
            }
        }
    }
    let mut video_active_model: video::ActiveModel = video_model.into();
    video_active_model.download_status = Set(status.into());
    video_active_model.path = Set(base_path.to_string_lossy().to_string());
    Ok(video_active_model)
}

/// 分发并执行分页下载任务，当且仅当所有分页成功下载或达到最大重试次数时返回 Ok，否则根据失败原因返回对应的错误
pub async fn dispatch_download_page(
    should_run: bool,
    video_model: &video::Model,
    page_models: Vec<page::Model>,
    base_path: &Path,
    cx: DownloadContext<'_>,
) -> Result<ExecutionStatus> {
    if !should_run {
        return Ok(ExecutionStatus::Skipped);
    }
    let child_semaphore = Semaphore::new(cx.config.concurrent_limit.page);
    let tasks = page_models
        .into_iter()
        .map(|page_model| download_page(video_model, page_model, &child_semaphore, base_path, cx))
        .collect::<FuturesUnordered<_>>();
    let (mut risk_control_related_error, mut target_status) = (None, STATUS_OK);
    let mut stream = tasks
        .take_while(|res| {
            match res {
                Ok(model) => {
                    // 该视频的所有分页的下载状态都会在此返回，需要根据这些状态确认视频层“分页下载”子任务的状态
                    // 在过去的实现中，此处仅仅根据 page_download_status 的最高标志位来判断，如果最高标志位是 true 则认为完成
                    // 这样会导致即使分页中有失败到 MAX_RETRY 的情况，视频层的分页下载状态也会被认为是 Succeeded，不够准确
                    // 新版本实现会将此处取值为所有子任务状态的最小值，这样只有所有分页的子任务全部成功时才会认为视频层的分页下载状态是 Succeeded
                    let page_download_status = model.download_status.try_as_ref().expect("download_status must be set");
                    let separate_status: [u32; 5] = PageStatus::from(*page_download_status).into();
                    for status in separate_status {
                        target_status = target_status.min(status);
                    }
                }
                Err(e) => {
                    tracing::error!("下载分页失败 (bvid: {}): {}", video_model.bvid, e);
                    if let Some(e) = e.downcast_ref::<BiliError>() {
                        if e.is_risk_control_related() {
                            risk_control_related_error = Some(e.clone());
                        }
                    }
                }
            }
            // 仅在发生风控时终止流，其它情况继续执行
            futures::future::ready(risk_control_related_error.is_none())
        })
        .filter_map(|res| futures::future::ready(res.ok()))
        .chunks(10);
    while let Some(models) = stream.next().await {
        update_pages_model(models, cx.connection).await?;
    }
    if let Some(e) = risk_control_related_error {
        bail!(e);
    }
    // 视频中“分页下载”任务的状态始终与所有分页的最小状态一致
    Ok(ExecutionStatus::Fixed(target_status))
}

/// 下载某个分页，未发生风控且正常运行时返回 Ok(Page::ActiveModel)，其中 status 字段存储了新的下载状态，发生风控时返回 DownloadAbortError
pub async fn download_page(
    video_model: &video::Model,
    page_model: page::Model,
    semaphore: &Semaphore,
    base_path: &Path,
    cx: DownloadContext<'_>,
) -> Result<page::ActiveModel> {
    let _permit = semaphore.acquire().await.context("acquire semaphore failed")?;
    let mut status = PageStatus::from(page_model.download_status);
    let separate_status = status.should_run();
    let is_single_page = video_model.single_page.context("single_page is null")?;

    // 判断是否为番剧和花絮（提前计算，用于后续逻辑）
    let is_bangumi =
        video_model.bangumi_id.is_some() || (video_model.source_id.is_some() && video_model.source_type == Some(1));
    // 检查 section_title 是否为非空字符串（花絮才有 section_title）
    let has_section_title = video_model
        .section_title
        .as_ref()
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    let is_bangumi_extra = is_bangumi
        && (has_section_title || video_model.episode_number.is_none() || video_model.episode_number == Some(0));

    // 添加调试日志
    tracing::info!(
        "video {}: bangumi_id={:?}, source_id={:?}, source_type={:?}, section_title={:?}, episode_number={:?}, is_bangumi={}, is_bangumi_extra={}",
        video_model.bvid,
        video_model.bangumi_id,
        video_model.source_id,
        video_model.source_type,
        video_model.section_title,
        video_model.episode_number,
        is_bangumi,
        is_bangumi_extra
    );

    // 未记录路径时填充，已经填充过路径时使用现有的
    let (base_path, base_name) = match &page_model.path {
        Some(old_video_path) if !old_video_path.is_empty() => {
            let old_video_path = Path::new(old_video_path);
            let old_video_filename = old_video_path
                .file_name()
                .context("invalid page path format")?
                .to_string_lossy();

            // 对于番剧花絮，需要重新生成 base_name（使用 page 模板）
            // 因为旧路径可能使用了 bangumi 模板（如 "3年Z组银八老师 - S01E01"）
            // 同时需要使用视频源的基础路径，而不是 video_model.path（包含剧集名）
            if is_bangumi_extra {
                // 使用视频源的基础路径，而不是 video_model.path
                let source_base_path = cx.video_source.path();
                // 如果有 section_title，将其作为子目录加入 source_base_path
                let base_path = if let Some(ref section_title) = video_model.section_title {
                    source_base_path.join(section_title)
                } else {
                    source_base_path.to_path_buf()
                };
                let format_args = page_format_args(video_model, &page_model, &cx.config.time_format);
                tracing::info!(
                    "video {} (bangumi extra): show_title={:?}, name={:?}, format_args.title={:?}",
                    video_model.bvid,
                    video_model.show_title,
                    video_model.name,
                    format_args.get("title")
                );
                let base_name = cx.template.path_safe_render("page", &format_args)?;
                tracing::info!(
                    "video {} (bangumi extra): rendered base_name={}, source_base_path={}",
                    video_model.bvid,
                    base_name,
                    source_base_path.display()
                );
                (base_path, base_name)
            } else if is_single_page {
                // 单页下的路径是 {base_path}/{base_name}.mp4
                (
                    old_video_path
                        .parent()
                        .context("invalid page path format")?
                        .to_path_buf(),
                    old_video_filename.trim_end_matches(".mp4").to_string(),
                )
            } else {
                // 多页下的路径是 {base_path}/Season 1/{base_name} - S01Exx.mp4
                (
                    old_video_path
                        .parent()
                        .and_then(|p| p.parent())
                        .context("invalid page path format")?
                        .to_path_buf(),
                    old_video_filename
                        .rsplit_once(" - ")
                        .context("invalid page path format")?
                        .0
                        .to_string(),
                )
            }
        }
        _ => {
            // 对于番剧花絮，需要使用视频源的基础路径，而不是 video_model.path（包含剧集名）
            let base_path = if is_bangumi
                && video_model
                    .section_title
                    .as_ref()
                    .map(|s| !s.is_empty())
                    .unwrap_or(false)
            {
                // 番剧花絮：使用视频源的基础路径 + section_title
                let source_base_path = cx.video_source.path();
                if let Some(ref section_title) = video_model.section_title {
                    source_base_path.join(section_title)
                } else {
                    source_base_path.to_path_buf()
                }
            } else {
                // 其他情况：使用 base_path，如果有 section_title 则作为子目录
                if let Some(ref section_title) = video_model.section_title {
                    base_path.join(section_title)
                } else {
                    base_path.to_path_buf()
                }
            };

            let (template_name, format_args) = if is_bangumi_extra {
                // 花絮/PV/预告使用 page 模板
                let format_args = page_format_args(video_model, &page_model, &cx.config.time_format);
                tracing::info!(
                    "video {}: is_bangumi_extra=true. show_title={:?}, name={:?}, format_args.title={:?}, section_title={:?}",
                    video_model.bvid,
                    video_model.show_title,
                    video_model.name,
                    format_args.get("title"),
                    video_model.section_title
                );
                ("page", format_args)
            } else if is_bangumi {
                // 正常番剧使用 bangumi 模板
                tracing::debug!(
                    "video {}: is_bangumi=true, using bangumi template. episode_number={:?}",
                    video_model.bvid,
                    video_model.episode_number
                );
                let api_title = if let Some(ref season_id) = video_model.season_id {
                    get_cached_season_title(cx.bili_client, season_id, CancellationToken::new()).await
                } else {
                    None
                };

                (
                    "bangumi",
                    bangumi_page_format_args(video_model, &page_model, &cx.config.time_format, api_title.as_deref()),
                )
            } else {
                // 普通视频使用 page 模板
                tracing::debug!("video {}: using page template", video_model.bvid);
                (
                    "page",
                    page_format_args(video_model, &page_model, &cx.config.time_format),
                )
            };

            let base_name = cx.template.path_safe_render(template_name, &format_args)?;
            tracing::info!(
                "video {}: rendered base_name={}, base_path={}",
                video_model.bvid,
                base_name,
                base_path.display()
            );
            (base_path, base_name)
        }
    };

    let (poster_path, video_path, nfo_path, danmaku_path, fanart_path, subtitle_path) =
        if is_single_page || is_bangumi_extra {
            // 单页视频或番剧花絮使用简单路径格式
            (
                base_path.join(format!("{}-poster.jpg", &base_name)),
                base_path.join(format!("{}.mp4", &base_name)),
                base_path.join(format!("{}.nfo", &base_name)),
                base_path.join(format!("{}.zh-CN.default.ass", &base_name)),
                Some(base_path.join(format!("{}-fanart.jpg", &base_name))),
                base_path.join(format!("{}.srt", &base_name)),
            )
        } else {
            // 多页视频使用 Season 1 + S01Exx 格式
            (
                base_path
                    .join("Season 1")
                    .join(format!("{} - S01E{:0>2}-thumb.jpg", &base_name, page_model.pid)),
                base_path
                    .join("Season 1")
                    .join(format!("{} - S01E{:0>2}.mp4", &base_name, page_model.pid)),
                base_path
                    .join("Season 1")
                    .join(format!("{} - S01E{:0>2}.nfo", &base_name, page_model.pid)),
                base_path
                    .join("Season 1")
                    .join(format!("{} - S01E{:0>2}.zh-CN.default.ass", &base_name, page_model.pid)),
                // 对于多页视频，会在上一步 fetch_video_poster 中获取剧集的 fanart，无需在此处下载单集的
                None,
                base_path
                    .join("Season 1")
                    .join(format!("{} - S01E{:0>2}.srt", &base_name, page_model.pid)),
            )
        };
    let dimension = match (page_model.width, page_model.height) {
        (Some(width), Some(height)) => Some(Dimension {
            width,
            height,
            rotate: 0,
        }),
        _ => None,
    };
    let page_info = PageInfo {
        cid: page_model.cid,
        duration: page_model.duration,
        dimension,
        ..Default::default()
    };
    let (res_1, res_2, res_3, res_4, res_5) = tokio::join!(
        // 下载分页封面
        fetch_page_poster(
            separate_status[0] && !cx.config.skip_option.no_poster,
            video_model,
            &page_model,
            poster_path,
            fanart_path,
            cx
        ),
        // 下载分页视频
        fetch_page_video(separate_status[1], video_model, &page_info, &video_path, cx),
        // 生成分页视频信息的 nfo
        generate_page_nfo(
            separate_status[2] && !cx.config.skip_option.no_video_nfo,
            video_model,
            &page_model,
            nfo_path,
            cx,
        ),
        // 下载分页弹幕
        fetch_page_danmaku(
            separate_status[3] && !cx.config.skip_option.no_danmaku,
            video_model,
            &page_info,
            danmaku_path,
            cx,
        ),
        // 下载分页字幕
        fetch_page_subtitle(
            separate_status[4] && !cx.config.skip_option.no_subtitle,
            video_model,
            &page_info,
            &subtitle_path,
            cx
        )
    );
    let results = [res_1.into(), res_2.into(), res_3.into(), res_4.into(), res_5.into()];
    status.update_status(&results);
    results
        .iter()
        .zip(["封面", "视频", "详情", "弹幕", "字幕"])
        .for_each(|(res, task_name)| match res {
            ExecutionStatus::Skipped => info!(
                "处理视频「{}」第 {} 页{}已成功过，跳过",
                &video_model.name, page_model.pid, task_name
            ),
            ExecutionStatus::Succeeded => info!(
                "处理视频「{}」第 {} 页{}成功",
                &video_model.name, page_model.pid, task_name
            ),
            ExecutionStatus::Ignored(e) => {
                error!(
                    "处理视频「{}」第 {} 页{}出现常见错误，已忽略：{:#}",
                    &video_model.name, page_model.pid, task_name, e
                )
            }
            ExecutionStatus::Failed(e) => error!(
                "处理视频「{}」第 {} 页{}失败：{:#}",
                &video_model.name, page_model.pid, task_name, e
            ),
            ExecutionStatus::Fixed(_) => unreachable!(),
        });
    for result in results {
        if let ExecutionStatus::Failed(e) = result {
            if let Ok(e) = e.downcast::<BiliError>() {
                if e.is_risk_control_related() {
                    bail!(e);
                }
            }
        }
    }
    let mut page_active_model: page::ActiveModel = page_model.into();
    page_active_model.download_status = Set(status.into());
    page_active_model.path = Set(Some(video_path.to_string_lossy().to_string()));
    Ok(page_active_model)
}

pub async fn fetch_page_poster(
    should_run: bool,
    video_model: &video::Model,
    page_model: &page::Model,
    poster_path: PathBuf,
    fanart_path: Option<PathBuf>,
    cx: DownloadContext<'_>,
) -> Result<ExecutionStatus> {
    if !should_run {
        return Ok(ExecutionStatus::Skipped);
    }
    let single_page = video_model.single_page.context("single_page is null")?;
    let url = if single_page {
        // 单页视频直接用视频的封面
        video_model.cover.as_str()
    } else {
        // 多页视频，如果单页没有封面，就使用视频的封面
        match &page_model.image {
            Some(url) => url.as_str(),
            None => video_model.cover.as_str(),
        }
    };
    cx.downloader
        .fetch(url, &poster_path, &cx.config.concurrent_limit.download)
        .await?;
    if let Some(fanart_path) = fanart_path {
        fs::copy(&poster_path, &fanart_path).await?;
    }
    Ok(ExecutionStatus::Succeeded)
}

pub async fn fetch_page_video(
    should_run: bool,
    video_model: &video::Model,
    page_info: &PageInfo,
    page_path: &Path,
    cx: DownloadContext<'_>,
) -> Result<ExecutionStatus> {
    if !should_run {
        return Ok(ExecutionStatus::Skipped);
    }
    let bili_video = Video::new(cx.bili_client, video_model.bvid.clone(), &cx.config.credential);
    let streams = bili_video
        .get_page_analyzer(page_info)
        .await?
        .best_stream(&cx.config.filter_option)?;
    match streams {
        BestStream::Mixed(mix_stream) => {
            cx.downloader
                .multi_fetch(
                    &mix_stream.urls(cx.config.cdn_sorting),
                    page_path,
                    &cx.config.concurrent_limit.download,
                )
                .await?
        }
        BestStream::VideoAudio {
            video: video_stream,
            audio: None,
        } => {
            cx.downloader
                .multi_fetch(
                    &video_stream.urls(cx.config.cdn_sorting),
                    page_path,
                    &cx.config.concurrent_limit.download,
                )
                .await?
        }
        BestStream::VideoAudio {
            video: video_stream,
            audio: Some(audio_stream),
        } => {
            cx.downloader
                .multi_fetch_and_merge(
                    &video_stream.urls(cx.config.cdn_sorting),
                    &audio_stream.urls(cx.config.cdn_sorting),
                    page_path,
                    &cx.config.concurrent_limit.download,
                )
                .await?
        }
    }
    Ok(ExecutionStatus::Succeeded)
}

pub async fn fetch_page_danmaku(
    should_run: bool,
    video_model: &video::Model,
    page_info: &PageInfo,
    danmaku_path: PathBuf,
    cx: DownloadContext<'_>,
) -> Result<ExecutionStatus> {
    if !should_run {
        return Ok(ExecutionStatus::Skipped);
    }
    let bili_video = Video::new(cx.bili_client, video_model.bvid.clone(), &cx.config.credential);
    bili_video
        .get_danmaku_writer(page_info)
        .await?
        .write(danmaku_path, &cx.config.danmaku_option)
        .await?;
    Ok(ExecutionStatus::Succeeded)
}

pub async fn fetch_page_subtitle(
    should_run: bool,
    video_model: &video::Model,
    page_info: &PageInfo,
    subtitle_path: &Path,
    cx: DownloadContext<'_>,
) -> Result<ExecutionStatus> {
    if !should_run {
        return Ok(ExecutionStatus::Skipped);
    }
    let bili_video = Video::new(cx.bili_client, video_model.bvid.clone(), &cx.config.credential);
    let subtitles = bili_video.get_subtitles(page_info).await?;
    let tasks = subtitles
        .into_iter()
        .map(|subtitle| async move {
            let path = subtitle_path.with_extension(format!("{}.srt", subtitle.lan));
            tokio::fs::write(path, subtitle.body.to_string()).await
        })
        .collect::<FuturesUnordered<_>>();
    tasks.try_collect::<Vec<()>>().await?;
    Ok(ExecutionStatus::Succeeded)
}

pub async fn generate_page_nfo(
    should_run: bool,
    video_model: &video::Model,
    page_model: &page::Model,
    nfo_path: PathBuf,
    cx: DownloadContext<'_>,
) -> Result<ExecutionStatus> {
    if !should_run {
        return Ok(ExecutionStatus::Skipped);
    }
    let single_page = video_model.single_page.context("single_page is null")?;
    let nfo = if single_page {
        NFO::Movie(video_model.to_nfo(cx.config.nfo_time_type))
    } else {
        NFO::Episode(page_model.to_nfo(cx.config.nfo_time_type))
    };
    generate_nfo(nfo, nfo_path).await?;
    Ok(ExecutionStatus::Succeeded)
}

pub async fn fetch_video_poster(
    should_run: bool,
    video_model: &video::Model,
    poster_path: PathBuf,
    fanart_path: PathBuf,
    cx: DownloadContext<'_>,
) -> Result<ExecutionStatus> {
    if !should_run {
        return Ok(ExecutionStatus::Skipped);
    }
    cx.downloader
        .fetch(&video_model.cover, &poster_path, &cx.config.concurrent_limit.download)
        .await?;
    fs::copy(&poster_path, &fanart_path).await?;
    Ok(ExecutionStatus::Succeeded)
}

pub async fn fetch_upper_face(
    should_run: bool,
    video_model: &video::Model,
    upper_face_path: PathBuf,
    cx: DownloadContext<'_>,
) -> Result<ExecutionStatus> {
    if !should_run {
        return Ok(ExecutionStatus::Skipped);
    }
    cx.downloader
        .fetch(
            &video_model.upper_face,
            &upper_face_path,
            &cx.config.concurrent_limit.download,
        )
        .await?;
    Ok(ExecutionStatus::Succeeded)
}

pub async fn generate_upper_nfo(
    should_run: bool,
    video_model: &video::Model,
    nfo_path: PathBuf,
    cx: DownloadContext<'_>,
) -> Result<ExecutionStatus> {
    if !should_run {
        return Ok(ExecutionStatus::Skipped);
    }
    generate_nfo(NFO::Upper(video_model.to_nfo(cx.config.nfo_time_type)), nfo_path).await?;
    Ok(ExecutionStatus::Succeeded)
}

pub async fn generate_video_nfo(
    should_run: bool,
    video_model: &video::Model,
    nfo_path: PathBuf,
    cx: DownloadContext<'_>,
) -> Result<ExecutionStatus> {
    if !should_run {
        return Ok(ExecutionStatus::Skipped);
    }
    // 如果视频属于番剧订阅，使用番剧元数据生成 NFO
    match video_model.bangumi_id {
        Some(bangumi_id) => {
            use bili_sync_entity::bangumi;
            use sea_orm::EntityTrait;
            match bangumi::Entity::find_by_id(bangumi_id).one(cx.connection).await {
                Ok(Some(bangumi_model)) => {
                    // 使用 helper 函数避免生命周期问题
                    generate_bangumi_nfo(&bangumi_model, nfo_path).await?;
                }
                _ => {
                    // 如果找不到番剧实体，回退到使用视频元数据
                    let nfo = NFO::TVShow(video_model.to_nfo(cx.config.nfo_time_type));
                    generate_nfo(nfo, nfo_path).await?;
                }
            }
        }
        None => {
            let nfo = NFO::TVShow(video_model.to_nfo(cx.config.nfo_time_type));
            generate_nfo(nfo, nfo_path).await?;
        }
    }
    Ok(ExecutionStatus::Succeeded)
}

async fn generate_bangumi_nfo(bangumi_model: &bili_sync_entity::bangumi::Model, nfo_path: PathBuf) -> Result<()> {
    let nfo = NFO::Bangumi(bangumi_model.to_nfo(crate::config::NFOTimeType::PubTime));
    generate_nfo(nfo, nfo_path).await
}

/// 创建 nfo_path 的父目录，然后写入 nfo 文件
async fn generate_nfo(nfo: NFO<'_>, nfo_path: PathBuf) -> Result<()> {
    if let Some(parent) = nfo_path.parent() {
        fs::create_dir_all(parent).await?;
    }
    fs::write(nfo_path, nfo.generate_nfo().await?.as_bytes()).await?;
    Ok(())
}
