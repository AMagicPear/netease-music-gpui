use std::{collections::HashMap, sync::Arc, time::Duration};

use gpui::Global;
// 供消息面板等外部调用方给 `MusicApi::message_*` 传参时命名客户端类型。
pub use ncm_api_rs::ApiClient;
use ncm_api_rs::{ApiResponse, NcmError, Query, create_client};
use tokio::runtime::Runtime;

use crate::models::{
    AudioQualityLevel, AudioSourceInfo, FeaturedActivity, HomepageBlock, LyricLine, MessageItem,
    Playlist, Privilege, RecommendEntry, RecommendedPlaylist, Song, SongComments, TrackId,
    UserProfile, VipInfo, parse_activities, parse_comment_page, parse_comments, parse_entries,
    parse_forwards, parse_homepage_blocks, parse_notices, parse_playlists, parse_private,
    parse_radar,
};

/// 「收藏到歌单」一次请求最多带多少首。歌单歌曲接口有单次数量上限，
/// 队列里可能有几百首（「收藏全部」），按批发才不会整批被服务端拒掉。
const COLLECT_BATCH: usize = 100;

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
        parse_comment_page(response.body, offset)
    }

    /// 歌单评论。响应形状与歌曲评论一致（`/api/v1/resource/comments/A_PL_0_{id}`），
    /// 解析直接复用同一个 [`parse_comment_page`]：offset 语义相同（普通评论分页，
    /// 热评只合并在第一页）。
    pub async fn playlist_comments(
        client: ApiClient,
        playlist_id: u64,
        offset: usize,
    ) -> Result<SongComments, String> {
        let response = Self::request(
            client.comment_playlist(
                &Query::new()
                    .param("id", &playlist_id.to_string())
                    .param("limit", "20")
                    .param("offset", &offset.to_string()),
            ),
        )
        .await?;
        parse_comment_page(response.body, offset)
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

    /// 推荐页顶部入口：官方把「每日推荐 + 多种风格日推」混排在一个区块里，
    /// 走新的首屏聚合接口，不必为每个入口单独发请求。
    pub async fn recommend_entries(client: ApiClient) -> Result<Vec<RecommendEntry>, String> {
        let response = Self::request(client.pc_rcmd_resource_show(&Query::new())).await?;
        Ok(parse_entries(&response.body))
    }

    /// 推荐歌单（推荐页的「推荐歌单」区块）。
    ///
    /// 官方 3.1.41 换成了 `/pc/personal/page/playlist/rcmd`：一次返回 12 条，
    /// creative 的深链里带 `referLog=HOMEPAGE_BLOCK_PLAYLIST_RCMD`。
    pub async fn recommended_playlists(
        client: ApiClient,
    ) -> Result<Vec<RecommendedPlaylist>, String> {
        let response = Self::request(client.personal_page_playlist_rcmd(&Query::new())).await?;
        Ok(parse_playlists(&response.body))
    }

    /// 雷达歌单：`/pc/customize/block/page` 一次给全歌单 id、封面、文案、播放量，
    /// 不需要再发歌单详情去补第一首歌封面。
    pub async fn radar_playlists(client: ApiClient) -> Result<Vec<RecommendedPlaylist>, String> {
        let response = Self::request(client.customize_block_page(&Query::new())).await?;
        Ok(parse_radar(&response.body))
    }

    /// 精选活动：运营位大图。官方主页面也会从首屏聚合接口里拿 banner 块，
    /// 这里用独立的 `/banner/get`，两者返回的是同一批运营内容。
    pub async fn featured_activities(client: ApiClient) -> Result<Vec<FeaturedActivity>, String> {
        // `banner()` 的默认 `type=0` 就是 `clientType=pc`。
        let response = Self::request(client.banner(&Query::new())).await?;
        Ok(parse_activities(&response.body))
    }

    /// 首页区块。游客也能取（未登录时是公开推荐），推荐歌单在未登录时用它兜底。
    pub async fn homepage_blocks(client: ApiClient) -> Result<Vec<HomepageBlock>, String> {
        let response = Self::request(client.homepage_block_page(&Query::new())).await?;
        Ok(parse_homepage_blocks(&response.body["data"]["blocks"]))
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

    /// 把歌曲收藏进歌单（官方 PC 客户端的「收藏到歌单」）。
    ///
    /// 接口一次能收的量有限，按 [`COLLECT_BATCH`] 首一批发。返回已成功的歌曲数和
    /// 第一个错误：分批之后可能前几批已经生效，调用方要按这个数字记账，不能当整批失败。
    pub async fn add_songs_to_playlist(
        client: ApiClient,
        playlist_id: u64,
        song_ids: Vec<u64>,
    ) -> (usize, Option<String>) {
        let mut added = 0;
        for chunk in song_ids.chunks(COLLECT_BATCH) {
            let tracks = chunk
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join(",");
            let result = Self::request(
                client.playlist_tracks(
                    &Query::new()
                        .param("op", "add")
                        .param("pid", &playlist_id.to_string())
                        .param("tracks", &tracks),
                ),
            )
            .await;
            match result {
                Ok(_) => added += chunk.len(),
                Err(message) => return (added, Some(message)),
            }
        }
        (added, None)
    }

    /// 新建歌单，返回新歌单。
    ///
    /// weapi 的响应有时把整个歌单放在 `playlist` 里、有时只给顶层 `id`，
    /// 两种都认：只剩 id 时用占位对象（封面等字段等下次刷新列表再补齐）。
    pub async fn create_playlist(client: &ApiClient, name: &str) -> Result<Playlist, String> {
        let mut response =
            Self::request(client.playlist_create(&Query::new().param("name", name))).await?;
        if let Ok(playlist) = serde_json::from_value::<Playlist>(response.body["playlist"].take())
            && playlist.id != 0
        {
            return Ok(playlist);
        }
        let id = response.body["id"]
            .as_u64()
            .filter(|id| *id != 0)
            .ok_or_else(|| "新建歌单没有返回歌单 id".to_string())?;
        Ok(Playlist::empty(id, name.to_owned()))
    }

    /// 依次把多首歌设为喜欢 / 取消喜欢，返回真正成功的 id 和第一个错误。
    ///
    /// 收藏进「我喜欢的音乐」要用它：那个歌单的内容就是账号的喜欢列表，
    /// 没有对应的歌单接口，只能一首首点亮。逐首发的代价是失败可能只发生在半途，
    /// 所以把成功的 id 带回去，让调用方只回滚没生效的那部分。
    pub async fn set_songs_liked(
        client: ApiClient,
        song_ids: Vec<u64>,
        liked: bool,
    ) -> (Vec<u64>, Option<String>) {
        let mut succeeded = Vec::with_capacity(song_ids.len());
        for song_id in song_ids {
            match Self::set_song_liked(client.clone(), song_id, liked).await {
                Ok(()) => succeeded.push(song_id),
                Err(message) => return (succeeded, Some(message)),
            }
        }
        (succeeded, None)
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

    /// 歌单详情：评论数、收藏数、曲目 id 全在这一步。
    /// 和 `playlist_songs` 分开，是因为详情很快就回来，歌曲要分批下；
    /// 拆开才能让头部（含「评论 N」）先渲染，不用等所有批次。
    pub async fn playlist_meta(client: ApiClient, playlist_id: u64) -> Result<Playlist, String> {
        let mut response = Self::request(
            client.playlist_detail(&Query::new().param("id", &playlist_id.to_string())),
        )
        .await?;
        serde_json::from_value(response.body["playlist"].take())
            .map_err(|error| format!("歌单详情格式无效：{error}"))
    }

    /// 按 `track_ids` 分批拉歌曲详情。
    pub async fn playlist_songs(
        client: ApiClient,
        playlist: Playlist,
    ) -> Result<Vec<Song>, String> {
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
        Ok(songs)
    }

    /// 详情 + 歌曲，等全部完成才返回。给不需要中间态的调用方用。
    pub async fn playlist(
        client: ApiClient,
        playlist_id: u64,
    ) -> Result<(Playlist, Vec<Song>), String> {
        let playlist = Self::playlist_meta(client.clone(), playlist_id).await?;
        let songs = Self::playlist_songs(client, playlist.clone()).await?;
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
    use crate::models::{CreativeTarget, PLAYLIST_BLOCK_CODE};
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
    #[ignore = "需要登录会话（或 COOKIE 环境变量）和网络连接"]
    fn recommend_entries_can_parse_real_results() {
        let api = MusicApi::from_env().expect("无法获取登录会话");
        let entries = api
            .runtime
            .block_on(MusicApi::recommend_entries(api.client.clone()))
            .expect("推荐入口失败");
        assert!(!entries.is_empty());
        // 官方首屏至少有一条每日推荐；风格日推随账号口味出现。
        let daily = entries
            .iter()
            .find(|entry| entry.sub_resource_type == "dailySong")
            .expect("推荐入口里应当有每日推荐");
        assert!(!daily.title.is_empty());
        assert!(daily.cover_url.is_some());
        for entry in &entries {
            assert!(entry.resource_id > 0);
            assert!(!entry.resource_type.is_empty());
        }
        // 顶部入口的封面也要能通过应用客户端下载（取色和卡片共用这份图）。
        let http = api.http_client();
        for entry in entries.iter().filter(|entry| entry.cover_url.is_some()) {
            let cover = crate::ui::assets::thumbnail_url(entry.cover_url.as_deref().unwrap(), 300);
            api.runtime.block_on(async {
                let response = http
                    .get(&cover, ().into(), true)
                    .await
                    .expect("入口封面下载失败");
                assert!(response.status().is_success(), "入口封面状态码异常");
            });
        }
    }

    #[test]
    #[ignore = "需要登录会话（或 COOKIE 环境变量）和网络连接"]
    fn recommended_playlists_can_parse_real_results() {
        let api = MusicApi::from_env().expect("无法获取登录会话");
        let playlists = api
            .runtime
            .block_on(MusicApi::recommended_playlists(api.client.clone()))
            .expect("推荐歌单失败");
        assert!(!playlists.is_empty());
        for playlist in &playlists {
            assert!(playlist.id > 0);
            assert!(!playlist.name.is_empty());
            assert!(playlist.pic_url.is_some());
        }
        // 推荐卡片点进去走的是同一条详情链路：摘要里的 id 必须能打开歌单。
        let (detail, songs) = api
            .runtime
            .block_on(MusicApi::playlist(api.client.clone(), playlists[0].id))
            .expect("歌单详情失败");
        assert_eq!(detail.name, playlists[0].name);
        assert!(!songs.is_empty());
    }

    #[test]
    #[ignore = "临时探针：把首页区块整个翻一遍"]
    fn probe_homepage_all_blocks() {
        let api = MusicApi::from_env().expect("无法获取登录会话");
        let mut cursor = String::new();
        for page in 0..2 {
            let query = Query::new().param("cursor", &cursor);
            let home = api
                .runtime
                .block_on(MusicApi::request(
                    api.client.clone().homepage_block_page(&query),
                ))
                .expect("首页失败");
            let data = &home.body["data"];
            let blocks = data["blocks"].as_array().cloned().unwrap_or_default();
            println!(
                "=== page {page} blocks={} hasMore={} ===",
                blocks.len(),
                data["hasMore"]
            );
            for block in &blocks {
                let code = block["blockCode"].as_str().unwrap_or("?");
                let creatives = block["creatives"].as_array().cloned().unwrap_or_default();
                let ui = &block["uiElement"];
                println!(
                    "[{code}] creatives={} uiKeys={:?} title={:?} sub={:?}",
                    creatives.len(),
                    ui.as_object()
                        .map(|ui| ui.keys().cloned().collect::<Vec<_>>()),
                    ui["mainTitle"]["title"],
                    ui["subTitle"]["title"],
                );
                if let Some(first) = creatives.first() {
                    let ui = &first["uiElement"];
                    println!(
                        "    first type={:?} title={:?} sub={} img={:?} action={:?} actionType={:?} keys={:?}",
                        first["creativeType"],
                        ui["mainTitle"]["title"],
                        serde_json::to_string(&ui["subTitle"]).unwrap(),
                        ui["image"]["imageUrl"],
                        first["action"],
                        first["actionType"],
                        ui.as_object()
                            .map(|ui| ui.keys().cloned().collect::<Vec<_>>()),
                    );
                }
            }
            let next = data["cursor"].as_str().unwrap_or("");
            if next.is_empty() {
                break;
            }
            cursor = next.to_owned();
        }
    }

    #[test]
    #[ignore = "需要登录会话（或 COOKIE 环境变量）和网络连接"]
    fn radar_playlists_can_parse_real_results() {
        let api = MusicApi::from_env().expect("无法获取登录会话");
        let playlists = api
            .runtime
            .block_on(MusicApi::radar_playlists(api.client.clone()))
            .expect("雷达歌单失败");
        assert!(!playlists.is_empty());
        let first = &playlists[0];
        assert!(first.id > 0);
        assert!(!first.name.is_empty());
        assert!(
            playlists.iter().all(|playlist| playlist.pic_url.is_some()),
            "雷达歌单每一张卡片都应当有封面"
        );
        // 文案里的 `|私人雷达` 后缀不该进卡片标题。
        assert!(!first.name.contains("|私人雷达"));
        // 雷达封面带 `imageView`/水印查询串，`thumbnail_url` 不能把它重新编码：
        // 参数值里的 `=` 被编码成 `%3D` 后 CDN 会直接 400，卡片和取色都会拿到默认图。
        let http = api.http_client();
        let cover = crate::ui::assets::thumbnail_url(first.pic_url.as_deref().unwrap(), 300);
        api.runtime.block_on(async {
            let response = http
                .get(&cover, ().into(), true)
                .await
                .expect("雷达封面下载失败");
            assert!(response.status().is_success(), "雷达封面状态码异常");
        });
    }

    #[test]
    #[ignore = "需要网络连接（不需要登录）"]
    fn featured_activities_can_parse_real_results() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let activities = runtime
            .block_on(MusicApi::featured_activities(create_client(None)))
            .expect("精选活动失败");
        assert!(!activities.is_empty());
        for activity in &activities {
            assert!(!activity.title.is_empty());
            assert!(activity.image_url.is_some());
        }
    }

    /// 首页区块游客也能取：卡片行的「推荐歌单」来自其中的
    /// `HOMEPAGE_BLOCK_PLAYLIST_RCMD`，条目带封面、播放量和歌单深链。
    #[test]
    #[ignore = "需要网络连接（不需要登录）"]
    fn homepage_blocks_can_parse_real_results() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let blocks = runtime
            .block_on(MusicApi::homepage_blocks(create_client(None)))
            .expect("首页区块失败");
        assert!(!blocks.is_empty());
        let card_block = blocks
            .iter()
            .find(|block| block.code == PLAYLIST_BLOCK_CODE)
            .expect("首页应当有推荐歌单区块");
        assert_eq!(card_block.title, "推荐歌单");
        assert!(!card_block.creatives.is_empty());
        assert!(
            card_block
                .creatives
                .iter()
                .any(|creative| matches!(creative.target, Some(CreativeTarget::Playlist(_))))
        );
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
