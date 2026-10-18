//! 推荐页（官方 3.1.41）的数据形状。
//!
//! 三块内容各有自己的接口：
//! - 顶部入口：`/pc/page/rcmd/resource/show` 里 `PC_HOMEPAGE_DAILY_MIX` 区块的
//!   `blockData.extInfo.pcRcmdDemoteItems`（每日推荐 + 多种风格日推混排）；
//! - 推荐歌单：`/pc/personal/page/playlist/rcmd` 的 `blockData.creatives`；
//! - 精选活动：`/banner/get` 的 `banners`；
//! - 雷达歌单：`/pc/customize/block/page` 里 `CUSTOMIZE_PLAYLIST_MGC` 区块。
//!
//! 歌单条目和首页区块共用同一套 creative 形状（图 + 标题 + `orpheus://` 深链 +
//! 播放量），所以解析统一走 [`super::homepage::parse_creatives`]。

use serde_json::Value;

use super::homepage::{CreativeTarget, HomepageCreative, parse_creatives};
use super::playlist::RecommendedPlaylist;

/// 顶部「每日推荐 + 风格日推」混排区块。
pub const DAILY_MIX_BLOCK_CODE: &str = "PC_HOMEPAGE_DAILY_MIX";
/// 私人雷达所在区块。
pub const RADAR_BLOCK_CODE: &str = "CUSTOMIZE_PLAYLIST_MGC";

/// `pcRcmdDemoteItems` 的一条：官方顶部入口。
///
/// 字段名字和接口保持一致，不在这里做展示层的裁剪；标题怎么取（`title` /
/// `coverText`）由 store 按入口类型决定。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecommendEntry {
    pub title: String,
    pub cover_text: String,
    pub cover_url: Option<String>,
    pub target_url: Option<String>,
    pub resource_type: String,
    pub sub_resource_type: String,
    pub resource_id: u64,
    /// 风格日推的标签 / 分类；每日推荐没有。
    pub tag_id: Option<u64>,
    pub category_id: Option<u64>,
    /// 风格日推点开时要带的示例歌曲。
    pub song_id: Option<u64>,
}

/// 精选活动的一张大图。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FeaturedActivity {
    pub title: String,
    pub image_url: Option<String>,
    pub link: Option<String>,
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
    // 老接口里 resourceId 是数字，新接口有时给字符串，两种都收。
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
    // 三样都没有的条目渲染不出来，跳过。
    if title.is_empty() && cover_text.is_empty() && cover_url.is_none() {
        return None;
    }
    let target_url = item["targetUrl"].as_str().map(str::to_owned);
    let ext = &item["extData"];
    Some(RecommendEntry {
        title,
        cover_text,
        cover_url,
        song_id: target_url
            .as_deref()
            .and_then(|url| query_u64(url, "songId")),
        target_url,
        resource_type: item["resourceType"].as_str().unwrap_or_default().to_owned(),
        sub_resource_type: item["subResourceType"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        resource_id,
        tag_id: ext["tagId"].as_u64(),
        category_id: ext["categoryId"].as_u64(),
    })
}

/// `orpheus://songrcmd?tab=style&...&songId=123` → 123。
fn query_u64(url: &str, key: &str) -> Option<u64> {
    url.split_once('?')?.1.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name == key).then(|| value.parse().ok()).flatten()
    })
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
        link: banner["url"].as_str().map(str::to_owned),
    })
}

/// 解析 `/pc/personal/page/playlist/rcmd` 的推荐歌单。
pub(crate) fn parse_playlists(value: &Value) -> Vec<RecommendedPlaylist> {
    playlists_from_creatives(&parse_creatives(&value["data"]["blockData"]["creatives"]))
}

/// 解析 `/pc/customize/block/page` 的雷达歌单。
///
/// 雷达条目的标题形如 `今天从《歌名》听起|私人雷达`，卡片上只保留 `|` 之前那截；
/// 普通歌单名里也可能出现 `|`，所以只在后缀确实是「私人雷达」时裁。
pub(crate) fn parse_radar(value: &Value) -> Vec<RecommendedPlaylist> {
    let mut playlists = value["data"]["blocks"]
        .as_array()
        .and_then(|blocks| {
            // 定制区块用 `code`，首页区块用 `blockCode`，两种都认。
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
        DAILY_MIX_BLOCK_CODE, parse_activities, parse_entries, parse_playlists, parse_radar,
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
                        "targetUrl": "orpheus://songrcmd?tab=default&resourceId=1540959188",
                        "resourceId": "1540959188",
                        "resourceType": "dailySongs",
                        "subResourceType": "dailySong",
                        "extData": {}
                    },
                    {
                        "title": "每日欧美流行推荐",
                        "coverText": "欧美流行日推",
                        "coverUrl": "http://p1.music.126.net/style.jpg",
                        "targetUrl": "orpheus://songrcmd?tab=style&categoryId=1000&tagId=10002&songId=1313591404",
                        "resourceId": "727950554",
                        "resourceType": "dailySongs",
                        "subResourceType": "style_dailySong",
                        "extData": { "tagId": 10002, "categoryId": 1000 }
                    }
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
        // 风格日推的 tag / category / song 都从接口里取，点开时不用再猜。
        assert_eq!(entries[1].tag_id, Some(10002));
        assert_eq!(entries[1].category_id, Some(1000));
        assert_eq!(entries[1].song_id, Some(1313591404));
    }

    #[test]
    fn parses_banners_with_image_and_title() {
        let activities = parse_activities(&serde_json::json!({
            "banners": [
                {
                    "typeTitle": "JJ LIN 林俊杰",
                    "bigImageUrl": "http://p1.music.126.net/big.jpg",
                    "imageUrl": "http://p1.music.126.net/small.jpg",
                    "url": "orpheus://album/123"
                },
                // 没图的运营位跳过。
                { "typeTitle": "没有图", "url": "orpheus://topic/1" }
            ]
        }));
        assert_eq!(activities.len(), 1);
        assert_eq!(activities[0].title, "JJ LIN 林俊杰");
        assert_eq!(
            activities[0].image_url.as_deref(),
            Some("http://p1.music.126.net/big.jpg")
        );
        assert_eq!(activities[0].link.as_deref(), Some("orpheus://album/123"));
    }

    #[test]
    fn parses_radar_playlists_and_strips_suffix() {
        let playlists = parse_radar(&serde_json::json!({
            "data": { "blocks": [{
                // 定制区块的 code 字段就叫 `code`，不是 `blockCode`。
                "code": "CUSTOMIZE_PLAYLIST_MGC",
                "creatives": [{
                    // 雷达的 action 是对象，不是老首页的字符串。
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
            }]}}
        }));
        assert_eq!(playlists.len(), 1);
        assert_eq!(playlists[0].id, 3015788101);
        assert_eq!(playlists[0].name, "走，去做梦！去睡觉！");
        assert_eq!(playlists[0].play_count, 18302);
    }
}
