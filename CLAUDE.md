# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

bili-sync is a Bilibili synchronization tool for NAS users. It downloads videos from favorites (收藏夹), collections (视频合集/视频列表), UP 主投稿, and watch later (稍后再看), with support for danmaku, subtitles, and media server-compatible NFO files.

**Architecture:** Rust backend (Axum + SeaORM + SQLite) + SvelteKit frontend

## Development Commands

```bash
# Justfile commands (run from project root)
just build-frontend        # Build frontend only
just build                 # Build frontend + release binary
just build-debug           # Build frontend + debug binary
just debug                 # Build frontend + run with cargo run
just clean                 # Clean build artifacts

# Frontend development (cd web/)
bun run dev                # Start dev server at http://localhost:5173
bun run build              # Build for production
bun run check              # Type check
bun run lint               # Lint

# Backend development
cargo test                 # Run tests
cargo run                  # Run debug build
```

**Database location:** `{CONFIG_DIR}/data.sqlite`

## Core Architecture

### VideoSource Abstraction

The core abstraction for video sources is the `VideoSource` trait in `crates/bili_sync/src/adapter/mod.rs`. Uses `enum_dispatch` for zero-overhead polymorphism.

**Video sources:**
- `Favorite` - 收藏夹
- `Collection` - 视频合集/视频列表 (type: Series=1, Season=2)
- `Submission` - UP主投稿 (supports regular and dynamic API)
- `WatchLater` - 稍后再看

**Key trait methods:**
- `refresh()` - Fetch video list from Bilibili API
- `filter_expr()` - Database filter for videos
- `should_take()` / `should_filter()` - Filter during iteration
- `path()` - Get save path
- `rule()` - Get filtering rules

### Download Workflow

Located in `crates/bili_sync/src/workflow.rs`. Main pipeline `process_video_source()`:

```
1. refresh_video_source()
   └─ Fetch video list from API
   └─ Insert new videos into database
   └─ Update latest_row_at timestamp

2. fetch_video_details()
   └─ Get detailed info (tags, pages, etc.)
   └─ Apply filtering rules
   └─ Mark invalid videos (404, 风控)

3. download_unprocessed_videos()
   └─ For each video:
       ├─ Download poster/fanart (多页视频 only)
       ├─ Generate tvshow.nfo (多页视频 only)
       ├─ Download UP主 avatar
       ├─ Generate person.nfo
       └─ For each page:
           ├─ Download video/audio (with FFmpeg merge if needed)
           ├─ Generate episode/TV nfo
           ├─ Download danmaku (ASS format)
           └─ Download subtitles (SRT format)
```

### Status Tracking

Download status is stored as bit-packed u32 for efficiency:
- **Video level:** 5 bits (poster, tvshow.nfo, upper avatar, person.nfo, page download)
- **Page level:** 5 bits (poster, video, nfo, danmaku, subtitle)
- Values: 0 = OK, 1-9 = RETRY, 10+ = FAILED

### File Layout

```
{video_source_path}/
├── poster.jpg           # Series poster (多页 only)
├── fanart.jpg           # Series fanart (多页 only)
├── tvshow.nfo           # Series metadata (多页 only)
├── {video_name}.mp4     # Single page video
├── {video_name}.nfo     # Single page metadata
├── {video_name}.zh-CN.default.ass  # Danmaku
└── Season 1/
    ├── {video_name} - S01E01.mp4
    ├── {video_name} - S01E01.nfo
    ├── {video_name} - S01E01-thumb.jpg
    ├── {video_name} - S01E01.zh-CN.default.ass
    └── {video_name} - S01E01.{lan}.srt  # Subtitles

{upper_path}/
└── {upper_id}/
    ├── folder.jpg       # UP主 avatar
    └── person.nfo       # UP主 info
```

### Bangumi (番剧) Handling

**Current behavior:** Bangumi/videos are identified by `redirect_url` field in `VideoInfo` (see `crates/bili_sync/src/utils/convert.rs:154-156`). Videos with `redirect_url = Some(...)` are marked as `valid = false` and skipped during download.

**To add bangumi support:** Requires adding a new `Bangumi` video source, database entity, adapter, and modifying the video validation logic.

## Module Structure

| Module | Purpose |
|--------|---------|
| `adapter/` | VideoSource trait implementations |
| `api/` | Axum REST API + WebSocket |
| `bilibili/` | Bilibili API client (WBI signing, credential refresh) |
| `config/` | Versioned configuration with change notifications |
| `database/` | Database connection setup |
| `downloader/` | Chunked parallel download with FFmpeg merge |
| `notifier/` | Telegram/Webhook notifications |
| `task/` | Scheduled task manager (cron/interval) |
| `utils/` | NFO generation, path templates, status helpers |
| `workflow.rs` | Core video processing pipeline |

## API Routes

All routes under `/api` with token-based authentication.

| Route Group | Endpoints |
|-------------|-----------|
| `/api/config` | GET, PUT - Application config |
| `/api/me` | `/favorites`, `/collections`, `/uppers` - User's Bilibili data |
| `/api/video-sources` | CRUD - Manage subscriptions |
| `/api/videos` | List, detail, reset - Downloaded videos |
| `/api/dashboard` | GET - Statistics |
| `/api/task` | POST `/download` - Manual trigger |
| `/api/ws` | WebSocket - Real-time logs, sysinfo, task status |

## Database Schema

**Main tables:**
- `favorite` - 收藏夹订阅
- `collection` - 合集/列表订阅 (type: 1=Series, 2=Season)
- `submission` - UP主投稿订阅 (use_dynamic_api flag)
- `watch_later` - 稍后再看 (singleton, id=1)
- `video` - Video metadata (foreign key to above tables)
- `page` - Video pages/segments
- `config` - Application configuration

**Key relationships:**
- `video` table has `favorite_id`, `collection_id`, `submission_id`, `watch_later_id` (one active)
- `page` table belongs to `video`

## Configuration System

Located in `crates/bili_sync/src/config/`. Uses `VersionedConfig` pattern:

- Thread-safe cached values with change notifications
- Support for interval (seconds) or cron scheduling
- Categories: Auth, Credential, Filter, Danmaku, Skip, Naming, Paths, Concurrency, Schedule, Notifiers

## Important Patterns

### Adding a New Video Source

1. Create entity in `crates/bili_sync_entity/src/entities/`
2. Create adapter in `crates/bili_sync/src/adapter/` implementing `VideoSource` trait
3. Add variant to `VideoSourceEnum` in `crates/bili_sync/src/adapter/mod.rs`
4. Add API routes in `crates/bili_sync/src/api/routes/video_sources/mod.rs` and `/me/mod.rs`
5. Add migration file
6. Update frontend types and UI

### Risk Control Detection

Bilibili API error code -352 indicates 风控. When detected, the download task automatically terminates to prevent account blocking. See `crates/bili_sync/src/bilibili/error.rs`.

### Stream Selection

Located in `crates/bili_sync/src/bilibili/analyzer.rs`. Selects best video/audio streams based on:
- User quality/codec preferences
- HDR/Dolby/Hi-Res filters
- Mixed vs separate streams (FFmpeg merge required for separate)
