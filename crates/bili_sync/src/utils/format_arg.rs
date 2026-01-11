use chrono::Datelike;
use serde_json::json;

/// 完全基于API的番剧标题提取，无硬编码回退逻辑
fn extract_series_title_with_context(
    video_model: &bili_sync_entity::video::Model,
    api_title: Option<&str>,
) -> Option<String> {
    // 只使用API提供的真实番剧标题，无回退逻辑
    if let Some(title) = api_title {
        // 标准化空格：将多个连续空格合并为单个空格，去除括号前的空格
        let normalized_title = title
            .split_whitespace() // 分割字符串，自动去除首尾空格并处理连续空格
            .collect::<Vec<_>>()
            .join(" ") // 用单个空格重新连接
            .replace(" （", "（") // 去除全角括号前的空格
            .replace(" (", "("); // 去除半角括号前的空格
        return Some(normalized_title);
    }

    // 如果没有API标题，记录警告并返回None
    tracing::debug!(
        "番剧视频 {} (BVID: {}) 缺少API标题，尝试从 video_model.name 提取",
        video_model.name,
        video_model.bvid
    );

    // 回退：从 video_model.name 中提取番剧标题
    // video_model.name 的格式可能是 "番剧名_集数" 或 "番剧名 集数"
    let name = video_model.name.trim();
    // 先尝试按空格分割（处理 "番剧名 集数" 格式）
    if let Some(pos) = name.find(' ') {
        return Some(name[..pos].trim().to_string());
    }
    // 再尝试按下划线分割（处理 "番剧名_集数" 格式）
    if let Some(pos) = name.find('_') {
        return Some(name[..pos].trim().to_string());
    }

    // 都没有，使用完整标题
    Some(name.to_string())
}

pub fn video_format_args(video_model: &bili_sync_entity::video::Model, time_format: &str) -> serde_json::Value {
    json!({
        "bvid": &video_model.bvid,
        "title": &video_model.name,
        "upper_name": &video_model.upper_name,
        "upper_mid": &video_model.upper_id,
        "pubtime": &video_model.pubtime.and_utc().format(time_format).to_string(),
        "fav_time": &video_model.favtime.and_utc().format(time_format).to_string(),
    })
}

pub fn page_format_args(
    video_model: &bili_sync_entity::video::Model,
    page_model: &bili_sync_entity::page::Model,
    time_format: &str,
) -> serde_json::Value {
    // 优先使用 show_title，如果没有则使用 name
    let display_title = video_model.show_title.as_ref().unwrap_or(&video_model.name);
    json!({
        "bvid": &video_model.bvid,
        "title": display_title,
        "name": &video_model.name,
        "upper_name": &video_model.upper_name,
        "upper_mid": &video_model.upper_id,
        "ptitle": &page_model.name,
        "pid": page_model.pid,
        "pubtime": video_model.pubtime.and_utc().format(time_format).to_string(),
        "fav_time": video_model.favtime.and_utc().format(time_format).to_string(),
    })
}

/// 从番剧标题中提取季度编号
fn extract_season_number(episode_title: &str) -> i32 {
    let title = episode_title.trim();

    // 移除开头的下划线（如果有）
    let title = title.strip_prefix('_').unwrap_or(title);

    // 查找季度标识的几种模式
    // 模式1: "第X季"
    if let Some(pos) = title.find("第") {
        let after_di = &title[pos + "第".len()..];
        if let Some(ji_pos) = after_di.find("季") {
            let season_str = &after_di[..ji_pos];
            // 尝试解析中文数字或阿拉伯数字
            match season_str {
                "一" => return 1,
                "二" => return 2,
                "三" => return 3,
                "四" => return 4,
                "五" => return 5,
                "六" => return 6,
                "七" => return 7,
                "八" => return 8,
                "九" => return 9,
                "十" => return 10,
                _ => {
                    // 尝试解析阿拉伯数字
                    if let Ok(season) = season_str.parse::<i32>() {
                        if season > 0 && season <= 50 {
                            return season;
                        }
                    }
                }
            }
        }
    }

    // 模式2: "Season X" 或 "season X"
    for pattern in ["Season ", "season "] {
        if let Some(pos) = title.find(pattern) {
            let after_season = &title[pos + pattern.len()..];
            // 找到第一个非数字字符的位置
            let season_end = after_season
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(after_season.len());
            let season_str = &after_season[..season_end];
            if let Ok(season) = season_str.parse::<i32>() {
                if season > 0 && season <= 50 {
                    return season;
                }
            }
        }
    }

    // 默认返回1
    1
}

/// 从视频标题中提取版本信息
fn extract_version_info(video_title: &str) -> String {
    let title = video_title.trim().strip_prefix('_').unwrap_or(video_title);

    // 如果标题很短且不包含常见的番剧标识符，可能是版本标识
    if title.len() <= 6 && !title.contains("第") && !title.contains("话") && !title.contains("集") {
        return title.to_string();
    }

    // 其他情况返回空字符串
    String::new()
}

/// 番剧专用的格式化函数，支持动态季度编号、集数编号等
pub fn bangumi_page_format_args(
    video_model: &bili_sync_entity::video::Model,
    page_model: &bili_sync_entity::page::Model,
    time_format: &str,
    api_title: Option<&str>,
) -> serde_json::Value {
    // 从数据库读取集数，如果没有则使用 page_model.pid
    let episode_number = video_model.episode_number.unwrap_or(page_model.pid);

    // 优先从标题中提取季度编号，如果提取失败则使用数据库中存储的值，最后默认为1
    let raw_season_number = match extract_season_number(&video_model.name) {
        1 => video_model.season_number.unwrap_or(1), // 如果从标题提取到1，可能是默认值，使用数据库值
        extracted => extracted,                      // 从标题提取到了明确的季度信息，使用提取的值
    } as u32;

    // 从发布时间提取年份
    let year = video_model.pubtime.year();

    // 提取番剧系列标题用于文件夹命名
    // 优先使用API标题，如果API获取失败则从 video_model.name 中提取
    let series_title = extract_series_title_with_context(video_model, api_title).unwrap_or_else(|| {
        tracing::debug!(
            "番剧视频 {} (BVID: {}) 无法提取标题，使用空字符串",
            video_model.name,
            video_model.bvid
        );
        String::new()
    });

    // 提取版本信息用于文件名区分
    let version_info = extract_version_info(&video_model.name);

    // 智能处理版本信息重复问题
    let final_version = if !version_info.is_empty() && page_model.name.trim() == version_info {
        String::new() // 避免重复，清空version字段
    } else {
        version_info
    };

    // 生成分辨率信息
    let resolution = match (page_model.width, page_model.height) {
        (Some(w), Some(h)) => format!("{}x{}", w, h),
        _ => "Unknown".to_string(),
    };

    // 内容类型判断
    let content_type = match video_model.category {
        1 => "动画",     // 动画分类
        177 => "纪录片", // 纪录片分类
        155 => "时尚",   // 时尚分类
        _ => "番剧",     // 默认为番剧
    };

    // 播出状态
    let status = if video_model.season_id.is_some() {
        "连载中" // 有season_id通常表示正在播出
    } else {
        "已完结" // 没有season_id可能表示已完结或单集
    };

    json!({
        "bvid": &video_model.bvid,
        "title": &video_model.name,
        "upper_name": &video_model.upper_name,
        "upper_mid": &video_model.upper_id,
        "ptitle": &page_model.name,
        "pid": episode_number,
        "pid_pad": format!("{:02}", episode_number),
        "season": raw_season_number,
        "season_pad": format!("{:02}", raw_season_number),
        "year": year,
        "studio": &video_model.upper_name,
        "actors": video_model.actors.as_deref().unwrap_or(""),
        "share_copy": video_model.share_copy.as_deref().unwrap_or(""),
        "category": video_model.category,
        "resolution": resolution,
        "content_type": content_type,
        "status": status,
        "ep_id": video_model.ep_id.as_deref().unwrap_or(""),
        "season_id": video_model.season_id.as_deref().unwrap_or(""),
        "pubtime": video_model.pubtime.and_utc().format(time_format).to_string(),
        "fav_time": video_model.favtime.and_utc().format(time_format).to_string(),
        "show_title": &video_model.name,
        "series_title": &series_title,
        "version": &final_version,
    })
}

/// 番剧专用的格式化函数（兼容旧版，无 api_title 参数）
#[allow(dead_code)]
pub fn bangumi_page_format_args_legacy(
    video_model: &bili_sync_entity::video::Model,
    page_model: &bili_sync_entity::page::Model,
    time_format: &str,
) -> serde_json::Value {
    bangumi_page_format_args(video_model, page_model, time_format, None)
}
