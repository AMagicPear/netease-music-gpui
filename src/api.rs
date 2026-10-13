use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use gpui::Global;
// 供消息面板等外部调用方给 `MusicApi::message_*` 传参时命名客户端类型。
pub use ncm_api_rs::ApiClient;
use ncm_api_rs::{ApiResponse, NcmError, Query, create_client};
use tokio::runtime::Runtime;

use crate::models::{
    AudioQualityLevel, AudioSourceInfo, LyricLine, MessageItem, Playlist, Privilege, Song,
    SongComment, SongComments, TrackId, UserProfile, VipInfo, parse_comments, parse_forwards,
    parse_notices, parse_private,
};

/// GPUI 负责界面，Tokio 负责 SDK 的网络请求。客户端和运行时在应用内复用。
pub struct MusicApi {
    pub client: ApiClient,
    pub runtime: Runtime,
    http: Arc<reqwest_client::ReqwestClient>,
    audio_http: reqwest::Client,
}

impl Global for MusicApi {}

impl MusicApi {
    pub fn http_client(&self) -> Arc<reqwest_client::ReqwestClient> {
        self.http.clone()
    }

    pub fn audio_http(&self) -> reqwest::Client {
        self.audio_http.clone()
    }

    /// 真实接口测试的入口：优先 `COOKIE` 环境变量，缺省时回落到应用已保存的
    /// 登录会话 —— 登录过一次本机应用后，`cargo test --ignored` 即可零配置运行。
    #[cfg(test)]
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let cookie = std::env::var("COOKIE")
            .ok()
            .filter(|cookie| !cookie.trim().is_empty())
            .or_else(|| {
                crate::persistence::Persistence::new()
                    .ok()
                    .and_then(|persistence| persistence.load_session().ok())
                    .flatten()
            })
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "没有 COOKIE 环境变量，也没有已保存的登录会话",
                )
            })?;

        Self::new(Some(cookie))
    }

    pub fn new(cookie: Option<String>) -> Result<Self, Box<dyn std::error::Error>> {
        let runtime = Runtime::new()?;
        let audio_http = reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(20))
            .build()?;
        let http = {
            // GPUI 适配器使用 gpui-pre-reqwest 分支，单独复用图片客户端与 Tokio 运行时。
            // 图片和音频的 HTTP 客户端都不向资源服务器发送账号 COOKIE。
            let _runtime = runtime.enter();
            Arc::new(reqwest_client::ReqwestClient::new())
        };
        Ok(Self {
            client: create_client(cookie),
            runtime,
            http,
            audio_http,
        })
    }

    pub async fn user_profile(client: ApiClient) -> Result<UserProfile, String> {
        let mut response = Self::request(client.user_account(&Query::new())).await?;
        let profile: UserProfile = serde_json::from_value(response.body["profile"].take())
            .map_err(|_| "账号资料格式无效，COOKIE 可能已失效".to_string())?;
        if profile.user_id == 0 || profile.nickname.trim().is_empty() {
            return Err("账号资料无效，COOKIE 可能已失效".into());
        }
        Ok(profile)
    }

    pub async fn vip_info(client: ApiClient, user_id: u64) -> Result<VipInfo, String> {
        let mut response =
            Self::request(client.vip_info(&Query::new().param("uid", &user_id.to_string())))
                .await?;
        serde_json::from_value(response.body["data"].take())
            .map_err(|error| format!("会员资料格式无效：{error}"))
    }

    async fn request(
        request: impl std::future::Future<Output = Result<ApiResponse, NcmError>>,
    ) -> Result<ApiResponse, String> {
        let response = tokio::time::timeout(Duration::from_secs(20), request)
            .await
            .map_err(|_| "API 请求超时".to_string())?
            .map_err(|_| "API 请求失败，请检查网络及 COOKIE".to_string())?;
        if response.status != 200 || response.body["code"].as_i64() != Some(200) {
            return Err(format!(
                "API 请求失败（状态 {}），请检查 COOKIE",
                response.status
            ));
        }
        Ok(response)
    }

    pub async fn song_counts(client: ApiClient, song_id: u64) -> [Result<u64, String>; 2] {
        let query = Query::new()
            .param("id", &song_id.to_string())
            .param("limit", "0");
        let (likes, comments) = tokio::join!(
            Self::request(client.song_red_count(&query)),
            Self::request(client.comment_music(&query)),
        );
        [
            likes.and_then(|response| {
                response.body["data"]["count"]
                    .as_u64()
                    .ok_or_else(|| "红心计数格式无效".into())
            }),
            comments.and_then(|response| {
                response.body["total"]
                    .as_u64()
                    .ok_or_else(|| "评论计数格式无效".into())
            }),
        ]
    }

    /// 单曲详情；歌单分批加载与网页 orpheus:// 唤起共用这个接口。
    pub async fn song_detail(client: ApiClient, song_id: u64) -> Result<Song, String> {
        let mut response =
            Self::request(client.song_detail(&Query::new().param("ids", &song_id.to_string())))
                .await?;
        let mut songs: Vec<Song> = serde_json::from_value(response.body["songs"].take())
            .map_err(|error| format!("歌曲详情格式无效：{error}"))?;
        // 权益拿不到不影响播放本身，音质判断会退回歌曲自带的变体字段。
        let privileges: Vec<Privilege> =
            serde_json::from_value(response.body["privileges"].take()).unwrap_or_default();
        attach_privileges(&mut songs, privileges);
        songs
            .into_iter()
            .next()
            .ok_or_else(|| format!("歌曲 {song_id} 不存在"))
    }

    pub async fn song_lyrics(client: ApiClient, song_id: u64) -> Result<Vec<LyricLine>, String> {
        let response =
            Self::request(client.lyric(&Query::new().param("id", &song_id.to_string()))).await?;
        Ok(song_lyric_lines(&response.body))
    }

    pub async fn song_comments(
        client: ApiClient,
        song_id: u64,
        offset: usize,
    ) -> Result<SongComments, String> {
        let response = Self::request(
            client.comment_music(
                &Query::new()
                    .param("id", &song_id.to_string())
                    .param("limit", "20")
                    .param("offset", &offset.to_string()),
            ),
        )
        .await?;
        // offset 是普通评论的分页位置，不能按合并热评后的条数推进。
        song_comment_page(response.body, offset)
    }

    pub async fn song_source(
        client: ApiClient,
        song_id: u64,
        quality: AudioQualityLevel,
    ) -> Result<AudioSourceInfo, String> {
        let response = Self::request(
            client.song_url_v1(
                &Query::new()
                    .param("id", &song_id.to_string())
                    .param("level", quality.api_level()),
            ),
        )
        .await?;
        song_source_info(&response.body, song_id)
    }

    pub async fn user_playlists(client: &ApiClient, user_id: u64) -> Result<Vec<Playlist>, String> {
        let mut playlists = Vec::new();
        loop {
            let mut response = Self::request(
                client.user_playlist(
                    &Query::new()
                        .param("uid", &user_id.to_string())
                        .param("limit", "100")
                        .param("offset", &playlists.len().to_string()),
                ),
            )
            .await?;
            let page: Vec<Playlist> = serde_json::from_value(response.body["playlist"].take())
                .map_err(|error| format!("歌单列表格式无效：{error}"))?;
            let more = response.body["more"].as_bool() == Some(true);
            let empty = page.is_empty();
            playlists.extend(page);
            if !more || empty {
                break;
            }
        }

        Ok(playlists)
    }

    pub async fn liked_song_ids(client: &ApiClient, user_id: u64) -> Result<Vec<u64>, String> {
        let like_query = Query::new().param("uid", &user_id.to_string());
        let mut likes = Self::request(client.likelist(&like_query)).await?;
        serde_json::from_value(likes.body["ids"].take())
            .map_err(|_| "喜欢的歌曲列表格式无效".to_string())
    }

    /// 私信会话列表（消息面板·私信 tab）。同时带回账号总未读数，用于 tab 角标。
    pub async fn private_messages(
        client: &ApiClient,
        user_id: u64,
    ) -> Result<(Vec<MessageItem>, u32), String> {
        let mut response =
            Self::request(client.msg_private(&Query::new().param("limit", "30"))).await?;
        let unread = response.body["newMsgCount"].as_u64().unwrap_or(0) as u32;
        let msgs = response.body["msgs"].take();
        Ok((parse_private(&msgs, user_id), unread))
    }

    /// 收到的评论（消息面板·评论 tab）。
    pub async fn comment_messages(
        client: &ApiClient,
        user_id: u64,
    ) -> Result<Vec<MessageItem>, String> {
        let mut response =
            Self::request(client.msg_comments(&Query::new().param("uid", &user_id.to_string())))
                .await?;
        let comments = response.body["comments"].take();
        Ok(parse_comments(&comments))
    }

    /// @我（消息面板·@我 tab）。同时带回总未读数。
    pub async fn mention_messages(client: &ApiClient) -> Result<(Vec<MessageItem>, u32), String> {
        let mut response =
            Self::request(client.msg_forwards(&Query::new().param("limit", "30"))).await?;
        let unread = response.body["newCount"].as_u64().unwrap_or(0) as u32;
        let forwards = response.body["forwards"].take();
        Ok((parse_forwards(&forwards), unread))
    }

    /// 系统通知（消息面板·通知 tab）。
    pub async fn notice_messages(client: &ApiClient) -> Result<Vec<MessageItem>, String> {
        let mut response =
            Self::request(client.msg_notices(&Query::new().param("limit", "30"))).await?;
        let notices = response.body["notices"].take();
        Ok(parse_notices(&notices))
    }

    /// 清空全部消息的未读计数（消息面板·一键全读）。
    ///
    /// 官方 web 客户端「清空全部新消息提醒」用的接口，无参数，四类计数一并清零。
    pub async fn clear_message_counts(client: &ApiClient) -> Result<(), String> {
        Self::request(client.msg_count_clear()).await.map(|_| ())
    }

    /// 喜欢 / 取消喜欢一首歌。成功时接口只回 code，不返回新状态。
    pub async fn set_song_liked(
        client: ApiClient,
        song_id: u64,
        liked: bool,
    ) -> Result<(), String> {
        Self::request(
            client.like(
                &Query::new()
                    .param("id", &song_id.to_string())
                    .param("like", if liked { "true" } else { "false" }),
            ),
        )
        .await?;
        Ok(())
    }

    /// 搜索单曲（type=1）。走 cloudsearch 接口：它返回的 songs 与 song_detail
    /// 同构（ar / al / dt），可以直接反序列化成 `Song`；老版 /search 用的是
    /// artists / album / duration，字段对不上。
    pub async fn search_songs(client: ApiClient, keywords: &str) -> Result<Vec<Song>, String> {
        let mut response = Self::request(
            client.cloudsearch(
                &Query::new()
                    .param("keywords", keywords)
                    .param("type", "1")
                    .param("limit", "30"),
            ),
        )
        .await?;
        let mut songs: Vec<Song> = serde_json::from_value(response.body["result"]["songs"].take())
            .map_err(|error| format!("搜索结果格式无效：{error}"))?;
        // 权益拿不到不影响列表本身，音质判断会退回歌曲自带的变体字段。
        let privileges: Vec<Privilege> =
            serde_json::from_value(response.body["result"]["privileges"].take())
                .unwrap_or_default();
        attach_privileges(&mut songs, privileges);
        Ok(songs)
    }

    /// 搜索歌单（type=1000）。和 `search_songs` 同一条 cloudsearch 链路，返回的 `playlists`
    /// 与歌单摘要同构，可以直接反序列化成 `Playlist` —— 摘要缺的字段由模型兜着。
    ///
    /// 一次取 50 条：虚拟表格只渲染可见行，多要几条只为滚动更顺手。
    /// 分页先不做，需要时按 `result.playlistCount` 接续加载。
    pub async fn search_playlists(
        client: ApiClient,
        keywords: &str,
    ) -> Result<Vec<Playlist>, String> {
        let mut response = Self::request(
            client.cloudsearch(
                &Query::new()
                    .param("keywords", keywords)
                    .param("type", "1000")
                    .param("limit", "50"),
            ),
        )
        .await?;
        let playlists: Vec<Playlist> =
            serde_json::from_value(response.body["result"]["playlists"].take())
                .map_err(|error| format!("歌单搜索结果格式无效：{error}"))?;
        Ok(playlists)
    }

    pub async fn playlist(
        client: ApiClient,
        playlist_id: u64,
    ) -> Result<(Playlist, Vec<Song>), String> {
        let mut response = Self::request(
            client.playlist_detail(&Query::new().param("id", &playlist_id.to_string())),
        )
        .await?;
        let playlist: Playlist = serde_json::from_value(response.body["playlist"].take())
            .map_err(|error| format!("歌单详情格式无效：{error}"))?;
        let mut songs = Vec::new();
        // 每个歌单沿用同一条链路，分批加载全部 trackIds。
        for tracks in playlist.track_ids.chunks(200) {
            let ids = tracks
                .iter()
                .map(|track| track.id.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let mut response =
                Self::request(client.song_detail(&Query::new().param("ids", &ids))).await?;
            let batch: Vec<Song> = serde_json::from_value(response.body["songs"].take())
                .map_err(|error| format!("歌曲详情格式无效：{error}"))?;
            // 权益拿不到不影响列表本身，音质判断会退回歌曲自带的变体字段。
            let privileges: Vec<Privilege> =
                serde_json::from_value(response.body["privileges"].take()).unwrap_or_default();
            let mut batch = ordered_playlist_songs(tracks, batch);
            attach_privileges(&mut batch, privileges);
            songs.extend(batch);
        }
        Ok((playlist, songs))
    }
}

fn song_lyric_lines(body: &serde_json::Value) -> Vec<LyricLine> {
    if body["nolyric"].as_bool() == Some(true) {
        return Vec::new();
    }
    let translations: HashMap<_, _> =
        parse_lrc(body["tlyric"]["lyric"].as_str().unwrap_or_default())
            .into_iter()
            .filter(|(_, text)| !text.is_empty())
            .collect();
    parse_lrc(body["lrc"]["lyric"].as_str().unwrap_or_default())
        .into_iter()
        .map(|(time, text)| LyricLine {
            time,
            text,
            translation: translations.get(&time).cloned(),
        })
        .collect()
}

fn parse_lrc(lrc: &str) -> Vec<(Duration, String)> {
    let mut lines = Vec::new();
    for line in lrc.lines() {
        let mut text = line.trim().trim_start_matches('\u{feff}');
        let mut times = Vec::new();
        while let Some(tagged) = text.strip_prefix('[') {
            let Some((tag, rest)) = tagged.split_once(']') else {
                break;
            };
            if let Some(time) = lrc_time(tag) {
                times.push(time);
            }
            text = rest;
        }
        // metadata 没有时间标签；空白时间行保留，用于清空当前歌词。
        for time in times {
            lines.push((time, text.trim().to_string()));
        }
    }
    lines.sort_by_key(|(time, _)| *time);
    lines
}

fn lrc_time(tag: &str) -> Option<Duration> {
    let (minutes, seconds) = tag.split_once(':')?;
    let (seconds, fraction) = seconds.split_once('.').unwrap_or((seconds, ""));
    if minutes.is_empty()
        || seconds.is_empty()
        || !minutes.bytes().all(|byte| byte.is_ascii_digit())
        || !seconds.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > 9
    {
        return None;
    }
    let minutes: u64 = minutes.parse().ok()?;
    let seconds: u64 = seconds.parse().ok()?;
    if seconds >= 60 {
        return None;
    }
    let nanos = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u32>().ok()? * 10u32.pow(9 - fraction.len() as u32)
    };
    Some(Duration::new(
        minutes.checked_mul(60)?.checked_add(seconds)?,
        nanos,
    ))
}

fn song_comment_page(body: serde_json::Value, offset: usize) -> Result<SongComments, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Comment {
        comment_id: u64,
        user: CommentUser,
        content: String,
        time: i64,
        liked_count: u64,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct CommentUser {
        nickname: String,
        avatar_url: String,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Page {
        total: u64,
        comments: Vec<Comment>,
        hot_comments: Option<Vec<Comment>>,
        more: bool,
    }
    let page: Page =
        serde_json::from_value(body).map_err(|error| format!("歌曲评论格式无效：{error}"))?;
    let mut seen = HashSet::new();
    let hot = if offset == 0 {
        page.hot_comments.unwrap_or_default()
    } else {
        Vec::new()
    };
    let comments = hot
        .into_iter()
        .chain(page.comments)
        .filter(|comment| seen.insert(comment.comment_id))
        .map(|comment| SongComment {
            id: comment.comment_id,
            nickname: comment.user.nickname,
            avatar_url: comment.user.avatar_url,
            content: comment.content,
            time: comment.time,
            liked_count: comment.liked_count,
        })
        .collect();
    Ok(SongComments {
        total: page.total,
        comments,
        more: page.more,
    })
}

fn song_source_info(body: &serde_json::Value, song_id: u64) -> Result<AudioSourceInfo, String> {
    let track = body["data"]
        .as_array()
        .and_then(|tracks| {
            tracks
                .iter()
                .find(|track| track["id"].as_u64() == Some(song_id))
        })
        .ok_or_else(|| "播放地址响应缺少当前歌曲".to_string())?;
    let url = track["url"]
        .as_str()
        .filter(|url| !url.is_empty())
        .ok_or_else(|| "这首歌暂无播放权限或已下架".to_string())?;
    let url = reqwest::Url::parse(url).map_err(|_| "歌曲播放地址无效".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("歌曲播放地址协议不受支持".into());
    }
    Ok(AudioSourceInfo {
        url: url.to_string(),
        cache_id: track["md5"]
            .as_str()
            .filter(|id| !id.is_empty())
            .map(str::to_owned),
        byte_len: track["size"].as_u64().filter(|size| *size > 0),
        duration: track["time"]
            .as_u64()
            .filter(|time| *time > 0)
            .map(Duration::from_millis),
        quality: track["level"]
            .as_str()
            .and_then(AudioQualityLevel::from_api_level),
    })
}

/// song_detail 的响应顺序不作为歌单顺序；缺失和额外的歌曲不补造。
fn ordered_playlist_songs(tracks: &[TrackId], songs: Vec<Song>) -> Vec<Song> {
    let mut by_id: HashMap<_, _> = songs.into_iter().map(|song| (song.id, song)).collect();
    tracks
        .iter()
        .filter_map(|track| by_id.remove(&track.id))
        .collect()
}

/// privileges 与 songs 是两个独立数组，只靠 id 对应，顺序不保证一致。
fn attach_privileges(songs: &mut [Song], privileges: Vec<Privilege>) {
    let mut by_id: HashMap<u64, Privilege> =
        privileges.into_iter().map(|item| (item.id, item)).collect();
    for song in songs {
        if let Some(privilege) = by_id.remove(&song.id) {
            song.privilege = Some(privilege);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::AsyncReadExt;
    use gpui::http_client::HttpClient;

    #[test]
    fn lyrics_expand_tags_sort_and_match_translation_by_time() {
        let body = serde_json::json!({
            "lrc": {"lyric": "\u{feff}[ar:歌手]\n[ti:歌曲]\n[offset:100]\n[00:02.50][00:01.5] 原文 \n[00:03]末行\n[00:04.000]\n[99:99]无效\n[18446744073709551615:01]溢出"},
            "tlyric": {"lyric": "[00:01.500][00:02.500] 译文 \n[00:03.0] \n[00:09]无对应原文"}
        });
        let lines = song_lyric_lines(&body);
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0].time, Duration::from_millis(1500));
        assert_eq!(lines[1].time, Duration::from_millis(2500));
        assert_eq!(lines[0].text, "原文");
        assert_eq!(lines[0].translation.as_deref(), Some("译文"));
        assert_eq!(lines[1].translation.as_deref(), Some("译文"));
        assert_eq!(lines[2].translation, None);
        assert_eq!(lines[3].text, "");
        assert_eq!(lrc_time("01:02.123"), Some(Duration::from_millis(62123)));
    }

    #[test]
    fn missing_and_instrumental_lyrics_are_empty() {
        for body in [
            serde_json::json!({}),
            serde_json::json!({"lrc": {"lyric": ""}}),
            serde_json::json!({"lrc": {"lyric": "[ti:歌曲]\n无时间文本"}}),
            serde_json::json!({"nolyric": true, "lrc": {"lyric": "[00:00]纯音乐"}}),
        ] {
            assert!(song_lyric_lines(&body).is_empty());
        }
    }

    #[test]
    fn comments_prioritize_hot_only_on_first_page_and_deduplicate() {
        let comment = |id, content| {
            serde_json::json!({
                "commentId": id, "user": {"nickname": "听众", "avatarUrl": "https://example.com/avatar"},
                "content": content, "time": 123456789, "likedCount": 42
            })
        };
        let body = serde_json::json!({
            "total": 100, "more": true,
            "hotComments": [comment(2, "热评"), comment(1, "另一条热评")],
            "comments": [comment(2, "重复"), comment(3, "普通评论"), comment(3, "重复")]
        });
        let first = song_comment_page(body.clone(), 0).unwrap();
        assert_eq!(first.total, 100);
        assert!(first.more);
        assert_eq!(
            first
                .comments
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            [2, 1, 3]
        );
        assert_eq!(first.comments[0].content, "热评");
        assert_eq!(first.comments[0].nickname, "听众");
        assert_eq!(first.comments[0].avatar_url, "https://example.com/avatar");
        assert_eq!(first.comments[0].time, 123456789);
        assert_eq!(first.comments[0].liked_count, 42);
        let next = song_comment_page(body, 20).unwrap();
        assert_eq!(
            next.comments.iter().map(|item| item.id).collect::<Vec<_>>(),
            [2, 3]
        );
        assert_eq!(next.comments[0].content, "重复");
        let empty = song_comment_page(
            serde_json::json!({"total": 0, "comments": [], "more": false}),
            0,
        )
        .unwrap();
        assert!(empty.comments.is_empty());
        assert!(!empty.more);
        assert!(song_comment_page(serde_json::json!({"comments": []}), 0).is_err());
    }

    #[test]
    fn playback_url_requires_matching_song_and_http_address() {
        let body = serde_json::json!({"data": [
            {"id": 1, "url": "https://example.com/song.mp3"},
            {"id": 2, "url": null},
            {"id": 3, "url": "file:///song.mp3"}
        ]});
        assert_eq!(
            song_source_info(&body, 1).unwrap().url,
            "https://example.com/song.mp3"
        );
        for id in [2, 3, 4] {
            assert!(song_source_info(&body, id).is_err());
        }
    }

    #[test]
    fn playback_quality_uses_actual_response_and_preserves_trial_duration() {
        let body = serde_json::json!({"data": [{
            "id": 1, "url": "https://example.com/song.flac", "size": 123456,
            "level": "lossless", "time": 30000, "md5": "content-version"
        }]});
        let source = song_source_info(&body, 1).unwrap();
        assert_eq!(source.quality, Some(AudioQualityLevel::Lossless));
        assert_eq!(source.duration, Some(Duration::from_secs(30)));
        assert_eq!(source.byte_len, Some(123456));
        assert_eq!(source.cache_id.as_deref(), Some("content-version"));
        for quality in AudioQualityLevel::ALL {
            assert_eq!(
                AudioQualityLevel::from_api_level(quality.api_level()),
                Some(quality)
            );
        }
        assert!(AudioQualityLevel::from_api_level("unknown").is_none());
    }

    #[test]
    fn privileges_attach_by_song_id_not_by_position() {
        let mut songs = vec![
            Song {
                id: 1,
                ..Default::default()
            },
            Song {
                id: 2,
                ..Default::default()
            },
        ];
        // 顺序故意和 songs 相反，且混入一条无关的权益。
        attach_privileges(
            &mut songs,
            vec![
                Privilege {
                    id: 3,
                    play_max_br_level: Some("hires".into()),
                    ..Default::default()
                },
                Privilege {
                    id: 2,
                    play_max_br_level: Some("lossless".into()),
                    ..Default::default()
                },
                Privilege {
                    id: 1,
                    play_max_br_level: Some("jymaster".into()),
                    ..Default::default()
                },
            ],
        );
        assert_eq!(
            songs[0].best_quality_level(),
            Some(AudioQualityLevel::JyMaster)
        );
        assert_eq!(
            songs[1].best_quality_level(),
            Some(AudioQualityLevel::Lossless)
        );
    }

    #[test]
    fn playlist_order_follows_track_ids_and_skips_unavailable_songs() {
        let tracks: Vec<_> = [3, 2, 1].into_iter().map(|id| TrackId { id }).collect();
        let songs = [5, 1, 3]
            .into_iter()
            .map(|id| Song {
                id,
                ..Default::default()
            })
            .collect();
        let ordered = ordered_playlist_songs(&tracks, songs);
        assert_eq!(
            ordered.iter().map(|song| song.id).collect::<Vec<_>>(),
            [3, 1]
        );
    }

    #[test]
    #[ignore = "需要登录会话（或 COOKIE 环境变量）和网络连接"]
    fn messages_can_load_from_real_api() {
        let api = MusicApi::from_env().expect("无法获取登录会话");
        let uid = api
            .runtime
            .block_on(MusicApi::user_profile(api.client.clone()))
            .expect("无法获取登录账号")
            .user_id;
        // 四个消息接口都能返回（空列表也算成功），条目字段可解析。
        let check = |name: &str, items: Vec<MessageItem>| {
            for message in &items {
                assert!(!message.nickname.is_empty(), "{name} 消息缺少发送者昵称");
            }
        };
        check(
            "私信",
            api.runtime
                .block_on(MusicApi::private_messages(&api.client, uid))
                .expect("私信")
                .0,
        );
        check(
            "评论",
            api.runtime
                .block_on(MusicApi::comment_messages(&api.client, uid))
                .expect("评论"),
        );
        check(
            "@我",
            api.runtime
                .block_on(MusicApi::mention_messages(&api.client))
                .expect("@我")
                .0,
        );
        check(
            "通知",
            api.runtime
                .block_on(MusicApi::notice_messages(&api.client))
                .expect("通知"),
        );
    }

    #[test]
    #[ignore = "需要登录会话（或 COOKIE 环境变量）和网络连接"]
    fn search_playlists_can_parse_real_results() {
        let api = MusicApi::from_env().expect("无法获取登录会话");
        let playlists = api
            .runtime
            .block_on(MusicApi::search_playlists(api.client.clone(), "周杰伦"))
            .expect("歌单搜索失败");
        assert!(!playlists.is_empty());
        for playlist in &playlists {
            assert!(playlist.id > 0);
            assert!(!playlist.name.is_empty());
            assert!(playlist.track_count > 0);
            // 搜索摘要不带创建时间，缺了要老实留空，而不是解析失败或编一个假日期。
            assert_eq!(playlist.create_time, None);
        }
        // 点进详情后，摘要里的歌单名要能和详情对上。
        let (detail, songs) = api
            .runtime
            .block_on(MusicApi::playlist(api.client.clone(), playlists[0].id))
            .expect("歌单详情失败");
        assert_eq!(detail.name, playlists[0].name);
        assert!(detail.create_time.is_some());
        assert!(!songs.is_empty());
    }

    #[test]
    #[ignore = "需要 COOKIE 环境变量和网络连接"]
    fn cookie_can_load_real_library() {
        let api = MusicApi::from_env().expect("无法读取 COOKIE 环境变量");
        let counts = api
            .runtime
            .block_on(MusicApi::song_counts(api.client.clone(), 186016));
        for count in counts {
            assert!(count.expect("歌曲互动计数请求失败") > 0);
        }
        let profile = api
            .runtime
            .block_on(MusicApi::user_profile(api.client.clone()))
            .expect("无法获取登录账号");
        assert!(profile.user_id > 0);
        assert!(!profile.nickname.is_empty());
        let vip = api
            .runtime
            .block_on(MusicApi::vip_info(api.client.clone(), profile.user_id))
            .expect("会员资料无法加载");
        if let Some(badge) =
            vip.badge_path(time::OffsetDateTime::now_utc().unix_timestamp() as u64 * 1000)
        {
            assert!(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("assets")
                    .join(badge)
                    .is_file()
            );
        }
        let playlists = api
            .runtime
            .block_on(MusicApi::user_playlists(&api.client, profile.user_id))
            .expect("无法加载真实音乐库");
        let likes = api
            .runtime
            .block_on(MusicApi::liked_song_ids(&api.client, profile.user_id))
            .expect("无法加载喜欢状态");
        let favorite_summary = playlists
            .iter()
            .find(|playlist| playlist.special_type == 5)
            .expect("缺少喜欢的音乐歌单");
        let (favorite, songs) = api
            .runtime
            .block_on(MusicApi::playlist(api.client.clone(), favorite_summary.id))
            .expect("无法加载收藏歌单");
        assert_eq!(songs.len(), favorite.track_ids.len());
        assert!(
            songs
                .iter()
                .zip(&favorite.track_ids)
                .all(|(song, track)| song.id == track.id)
        );
        assert!(songs.iter().all(|song| likes.contains(&song.id)));
        // 普通歌单也走相同接口，并显示 API 返回的创建者。
        for summary in [
            playlists.iter().find(|playlist| {
                playlist.special_type != 5 && playlist.creator.user_id == profile.user_id
            }),
            playlists
                .iter()
                .filter(|playlist| playlist.creator.user_id != profile.user_id)
                .min_by_key(|playlist| playlist.track_count),
        ]
        .into_iter()
        .flatten()
        {
            let (playlist, tracks) = api
                .runtime
                .block_on(MusicApi::playlist(api.client.clone(), summary.id))
                .expect("无法加载普通歌单");
            assert_eq!(playlist.id, summary.id);
            assert_eq!(playlist.name, summary.name);
            assert_eq!(playlist.creator.user_id, summary.creator.user_id);
            assert!(
                tracks
                    .iter()
                    .all(|song| playlist.track_ids.iter().any(|track| track.id == song.id))
            );
        }
        // 使用与应用相同的客户端验证真实头像、歌单封面和歌曲封面可下载。
        let http = api.http_client();
        let cover = songs
            .iter()
            .find_map(|song| song.al.pic_url.as_deref())
            .expect("缺少歌曲封面");
        let thumbnail = crate::ui::assets::thumbnail_url(cover, 72);
        let mut downloaded_sizes = Vec::new();
        for url in [
            &profile.avatar_url,
            favorite.cover_img_url.as_deref().expect("缺少歌单封面"),
            cover,
            &thumbnail,
        ] {
            api.runtime.block_on(async {
                tokio::time::timeout(Duration::from_secs(20), async {
                    let mut response = http.get(url, ().into(), true).await.expect("图片下载失败");
                    assert!(response.status().is_success());
                    assert!(
                        response.headers()["content-type"]
                            .to_str()
                            .unwrap()
                            .starts_with("image/")
                    );
                    let mut bytes = Vec::new();
                    response
                        .body_mut()
                        .read_to_end(&mut bytes)
                        .await
                        .expect("图片读取失败");
                    assert!(!bytes.is_empty());
                    downloaded_sizes.push(bytes.len());
                })
                .await
                .expect("图片下载超时");
            });
        }
        assert!(
            downloaded_sizes[3] < downloaded_sizes[2],
            "缩略图应比原图小"
        );
        eprintln!(
            "歌曲封面：原图 {} 字节，72px 缩略图 {} 字节",
            downloaded_sizes[2], downloaded_sizes[3]
        );
    }
}

#[cfg(test)]
mod temp_probe {
    use super::*;

    #[test]
    #[ignore = "临时探针"]
    fn probe_notices_raw() {
        let api = MusicApi::from_env().expect("会话");
        let response = api
            .runtime
            .block_on(api.client.msg_notices(&Query::new().param("limit", "30")))
            .unwrap();
        for (i, item) in response.body["notices"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let payload = item["notice"].as_str().unwrap_or("");
            let value: serde_json::Value = serde_json::from_str(payload).unwrap_or_default();
            // 只看 type != 6 的，找「赞评论」以外的类型。
            if value["type"].as_i64() == Some(6) {
                continue;
            }
            println!(
                "=== notices[{i}] type={} ===\n{}",
                value["type"],
                serde_json::to_string_pretty(item).unwrap()
            );
        }
    }

    #[test]
    #[ignore = "临时探针：会真的清空服务端未读计数"]
    fn probe_msg_count_clear() {
        let api = MusicApi::from_env().expect("会话");
        let response = api
            .runtime
            .block_on(api.client.msg_count_clear())
            .expect("清空未读计数失败");
        println!("{}", serde_json::to_string_pretty(&response.body).unwrap());
    }
}
