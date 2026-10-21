//! 真实接口回归默认忽略，显式运行时读取 COOKIE 或已保存的会话。

use super::*;
use crate::models::{CreativeTarget, MessageItem, PLAYLIST_BLOCK_CODE};
use futures::AsyncReadExt;
use gpui::http_client::HttpClient;

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
fn daily_songs_can_parse_real_results() {
    let api = MusicApi::from_env().expect("无法获取登录会话");
    let songs = api
        .runtime
        .block_on(MusicApi::daily_songs(api.client.clone()))
        .expect("每日推荐失败");
    assert!(!songs.is_empty());
    for song in &songs {
        assert!(song.id > 0);
        assert!(!song.name.is_empty());
        assert!(song.al.id > 0);
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
#[ignore = "需要登录会话（或 COOKIE 环境变量）和网络连接"]
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
