//! 推荐页接口解析；歌单条目复用首页的 creative 解析。

use serde_json::Value;

use super::homepage::{CreativeTarget, HomepageCreative, parse_creatives};
use super::playlist::RecommendedPlaylist;
use super::song::{Song, attach_privileges};

/// 顶部「每日推荐 + 风格日推」混排区块。
pub const DAILY_MIX_BLOCK_CODE: &str = "PC_HOMEPAGE_DAILY_MIX";
/// 私人雷达所在区块。
pub const RADAR_BLOCK_CODE: &str = "CUSTOMIZE_PLAYLIST_MGC";

/// `pcRcmdDemoteItems` 条目；标题与说明的取舍由 store 决定。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecommendEntry {
    pub title: String,
    pub cover_text: String,
    pub cover_url: Option<String>,
    pub resource_type: String,
    pub sub_resource_type: String,
    pub resource_id: u64,
}

/// 精选活动的一张大图。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FeaturedActivity {
    pub title: String,
    pub image_url: Option<String>,
}

/// 解析 `/pc/page/rcmd/resource/show` 里的顶部入口。
pub(crate) fn parse_entries(value: &Value) -> Vec<RecommendEntry> {
    value["data"]["blocks"]
        .as_array()
        .and_then(|blocks| {
            blocks
                .iter()
                .find(|block| block["blockCode"] == DAILY_MIX_BLOCK_CODE)
        })
        .and_then(|block| block["blockData"]["extInfo"]["pcRcmdDemoteItems"].as_array())
        .map(|items| items.iter().filter_map(parse_entry).collect())
        .unwrap_or_default()
}

fn parse_entry(item: &Value) -> Option<RecommendEntry> {
    // resourceId 同时存在数字和字符串两种响应形状。
    let resource_id = item["resourceId"]
        .as_u64()
        .or_else(|| item["resourceId"].as_str().and_then(|id| id.parse().ok()))?;
    let title = item["title"].as_str().unwrap_or_default().trim().to_owned();
    let cover_text = item["coverText"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_owned();
    let cover_url = item["coverUrl"]
        .as_str()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_owned);
    if title.is_empty() && cover_text.is_empty() && cover_url.is_none() {
        return None;
    }
    Some(RecommendEntry {
        title,
        cover_text,
        cover_url,
        resource_type: item["resourceType"].as_str().unwrap_or_default().to_owned(),
        sub_resource_type: item["subResourceType"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        resource_id,
    })
}

/// 每日推荐歌曲：`data.dailySongs` 是歌曲数组，权益可能内嵌在每首歌里，
/// 也可能单独放在 `data.privileges`，两种形状都合并。
pub(crate) fn parse_daily_songs(value: &Value) -> Result<Vec<Song>, String> {
    let data = &value["data"];
    let mut songs: Vec<Song> = serde_json::from_value(data["dailySongs"].clone())
        .map_err(|error| format!("每日推荐格式无效：{error}"))?;
    let privileges = serde_json::from_value(data["privileges"].clone()).unwrap_or_default();
    attach_privileges(&mut songs, privileges);
    Ok(songs)
}

/// 解析 `/banner/get` 的 `banners`。只要带图和标题的，运营位没图时直接跳过。
pub(crate) fn parse_activities(value: &Value) -> Vec<FeaturedActivity> {
    value["banners"]
        .as_array()
        .map(|banners| banners.iter().filter_map(parse_activity).collect())
        .unwrap_or_default()
}

fn parse_activity(banner: &Value) -> Option<FeaturedActivity> {
    let title = banner["typeTitle"].as_str().unwrap_or_default().trim();
    let image_url = banner["bigImageUrl"]
        .as_str()
        .or_else(|| banner["imageUrl"].as_str())
        .map(str::trim)
        .filter(|url| !url.is_empty());
    if title.is_empty() || image_url.is_none() {
        return None;
    }
    Some(FeaturedActivity {
        title: title.to_owned(),
        image_url: image_url.map(str::to_owned),
    })
}

/// 解析 `/pc/personal/page/playlist/rcmd` 的推荐歌单。
pub(crate) fn parse_playlists(value: &Value) -> Vec<RecommendedPlaylist> {
    playlists_from_creatives(&parse_creatives(&value["data"]["blockData"]["creatives"]))
}

/// 解析 `/pc/customize/block/page` 的雷达歌单。
///
/// 只裁掉确实为「私人雷达」的后缀，保留普通歌单名中的 `|`。
pub(crate) fn parse_radar(value: &Value) -> Vec<RecommendedPlaylist> {
    let mut playlists = value["data"]["blocks"]
        .as_array()
        .and_then(|blocks| {
            // 定制区块和首页区块使用不同的 code 字段。
            blocks.iter().find(|block| {
                block["blockCode"] == RADAR_BLOCK_CODE || block["code"] == RADAR_BLOCK_CODE
            })
        })
        .map(|block| playlists_from_creatives(&parse_creatives(&block["creatives"])))
        .unwrap_or_default();
    for playlist in &mut playlists {
        playlist.name = strip_radar_suffix(&playlist.name);
    }
    playlists
}

fn strip_radar_suffix(name: &str) -> String {
    match name.split_once('|') {
        Some((head, tail)) if tail.trim() == "私人雷达" => head.trim().to_owned(),
        _ => name.to_owned(),
    }
}

pub(crate) fn playlists_from_creatives(creatives: &[HomepageCreative]) -> Vec<RecommendedPlaylist> {
    creatives
        .iter()
        .filter_map(|creative| {
            let CreativeTarget::Playlist(id) = creative.target.as_ref()? else {
                return None;
            };
            if creative.title.trim().is_empty() {
                return None;
            }
            Some(RecommendedPlaylist {
                id: *id,
                name: creative.title.clone(),
                pic_url: creative.image_url.clone(),
                play_count: creative.play_count.unwrap_or(0),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        DAILY_MIX_BLOCK_CODE, parse_activities, parse_daily_songs, parse_entries, parse_playlists,
        parse_radar, strip_radar_suffix,
    };

    #[test]
    fn parses_daily_mix_entries() {
        let entries = parse_entries(&serde_json::json!({
            "data": { "blocks": [{
                "blockCode": DAILY_MIX_BLOCK_CODE,
                "blockData": { "extInfo": { "pcRcmdDemoteItems": [
                    {
                        "title": "每日推荐 | 你的多种喜好推荐",
                        "coverText": "29",
                        "coverUrl": "http://p1.music.126.net/daily.webp",
                        "resourceId": "1540959188",
                        "resourceType": "dailySongs",
                        "subResourceType": "dailySong"
                    },
                    {
                        "title": "每日欧美流行推荐",
                        "coverText": "欧美流行日推",
                        "coverUrl": "http://p1.music.126.net/style.jpg",
                        "resourceId": 727950554,
                        "resourceType": "dailySongs",
                        "subResourceType": "style_dailySong"
                    },
                    { "resourceId": "invalid", "title": "无效标识" },
                    { "resourceId": 1, "title": "  ", "coverUrl": "  " }
                ]}}
            }]}
        }));
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].resource_id, 1540959188);
        assert_eq!(entries[0].sub_resource_type, "dailySong");
        assert_eq!(entries[0].cover_text, "29");
        assert_eq!(
            entries[0].cover_url.as_deref(),
            Some("http://p1.music.126.net/daily.webp")
        );
        assert_eq!(entries[1].resource_id, 727950554);
        assert_eq!(entries[1].cover_text, "欧美流行日推");
    }

    #[test]
    fn parses_daily_songs_and_attaches_privileges_by_id() {
        let songs = parse_daily_songs(&serde_json::json!({
            "data": {
                "dailySongs": [
                    {"id": 1, "name": "第一首", "ar": [], "al": {"id": 3, "name": "专辑"}, "dt": 1000},
                    {"id": 2, "name": "第二首", "ar": [], "al": {"id": 4, "name": "专辑"}, "dt": 2000}
                ],
                "privileges": [{"id": 2, "playMaxBrLevel": "lossless"}]
            }
        }))
        .unwrap();
        assert_eq!(songs.len(), 2);
        assert_eq!(songs[0].name, "第一首");
        assert!(songs[0].privilege.is_none());
        assert_eq!(
            songs[1]
                .privilege
                .as_ref()
                .and_then(|privilege| privilege.play_max_br_level.as_deref()),
            Some("lossless")
        );
        // 缺少 dailySongs 时不能静默当成「今天没有推荐」。
        assert!(parse_daily_songs(&serde_json::json!({"data": {}})).is_err());
    }

    #[test]
    fn parses_banners_with_image_and_title() {
        let activities = parse_activities(&serde_json::json!({
            "banners": [
                {
                    "typeTitle": "JJ LIN 林俊杰",
                    "bigImageUrl": "http://p1.music.126.net/big.jpg",
                    "imageUrl": "http://p1.music.126.net/small.jpg"
                },
                { "typeTitle": "没有图" },
                { "typeTitle": " ", "imageUrl": "http://p1.music.126.net/small.jpg" }
            ]
        }));
        assert_eq!(activities.len(), 1);
        assert_eq!(activities[0].title, "JJ LIN 林俊杰");
        assert_eq!(
            activities[0].image_url.as_deref(),
            Some("http://p1.music.126.net/big.jpg")
        );
    }

    #[test]
    fn parses_radar_playlists_and_strips_suffix() {
        let playlists = parse_radar(&serde_json::json!({
            "data": { "blocks": [{
                "code": "CUSTOMIZE_PLAYLIST_MGC",
                "creatives": [{
                    "action": { "clickAction": { "targetUrl": "orpheus://playlist/3136952023?type=100&uiPlaylistType=MGC" } },
                    "uiElement": {
                        "mainTitle": { "title": "今天从《我的灵魂是个哑巴》听起|私人雷达" },
                        "images": [{ "imageUrl": "http://p1.music.126.net/radar.jpg" }]
                    },
                    "resources": [{
                        "resourceId": "3136952023",
                        "resourceExtInfo": { "playCount": 18302 }
                    }]
                }]
            }]}
        }));
        assert_eq!(playlists.len(), 1);
        assert_eq!(playlists[0].id, 3136952023);
        assert_eq!(playlists[0].name, "今天从《我的灵魂是个哑巴》听起");
        assert_eq!(
            playlists[0].pic_url.as_deref(),
            Some("http://p1.music.126.net/radar.jpg")
        );
        assert_eq!(playlists[0].play_count, 18302);
        assert_eq!(
            strip_radar_suffix(" 普通歌单 | 其他后缀"),
            " 普通歌单 | 其他后缀"
        );
    }

    #[test]
    fn parses_personal_playlist_creatives() {
        let playlists = parse_playlists(&serde_json::json!({
            "data": { "blockData": { "creatives": [{
                "action": "orpheus://playlist/3015788101?autoplay=0&referLog=HOMEPAGE_BLOCK_PLAYLIST_RCMD",
                "uiElement": {
                    "mainTitle": { "title": "走，去做梦！去睡觉！" },
                    "image": { "imageUrl": "http://p4.music.126.net/a.jpg" }
                },
                "resources": [{ "resourceExtInfo": { "playCount": 18302 } }]
            }, {
                "action": "orpheus://song/1",
                "uiElement": { "mainTitle": { "title": "单曲" } }
            }, {
                "action": "orpheus://playlist/2",
                "uiElement": { "mainTitle": { "title": " " } }
            }]}}
        }));
        assert_eq!(playlists.len(), 1);
        assert_eq!(playlists[0].id, 3015788101);
        assert_eq!(playlists[0].name, "走，去做梦！去睡觉！");
        assert_eq!(playlists[0].play_count, 18302);
    }
}
