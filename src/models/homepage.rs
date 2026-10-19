//! 首页区块及推荐页共用的 creative 解析。区块按 code 匹配，顺序随账号变化。

use serde_json::Value;

/// 「推荐歌单」区块的 code：推荐页卡片行用它。
pub const PLAYLIST_BLOCK_CODE: &str = "HOMEPAGE_BLOCK_PLAYLIST_RCMD";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct HomepageBlock {
    pub code: String,
    pub title: String,
    pub creatives: Vec<HomepageCreative>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct HomepageCreative {
    pub title: String,
    pub image_url: Option<String>,
    pub target: Option<CreativeTarget>,
    pub play_count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CreativeTarget {
    /// 歌单：`orpheus://playlist/{id}`。
    Playlist(u64),
    /// 未识别的深链原样保留。
    Link(String),
}

/// 解析响应里的 `data.blocks`。
pub(crate) fn parse_blocks(blocks: &Value) -> Vec<HomepageBlock> {
    blocks
        .as_array()
        .map(|blocks| blocks.iter().map(parse_block).collect())
        .unwrap_or_default()
}

fn parse_block(block: &Value) -> HomepageBlock {
    HomepageBlock {
        code: text(&block["blockCode"]),
        title: text(&block["uiElement"]["subTitle"]["title"]),
        creatives: parse_creatives(&block["creatives"]),
    }
}

/// 首页、推荐歌单和雷达接口共用此形状。
pub(crate) fn parse_creatives(creatives: &Value) -> Vec<HomepageCreative> {
    creatives
        .as_array()
        .map(|creatives| creatives.iter().filter_map(parse_creative).collect())
        .unwrap_or_default()
}

fn parse_creative(creative: &Value) -> Option<HomepageCreative> {
    let title = text(&creative["uiElement"]["mainTitle"]["title"]);
    // 首页用 image，雷达用 images 数组。
    let image_url = creative["uiElement"]["image"]["imageUrl"]
        .as_str()
        .or_else(|| creative["uiElement"]["images"][0]["imageUrl"].as_str())
        .map(str::to_owned);
    if title.is_empty() && image_url.is_none() {
        return None;
    }
    Some(HomepageCreative {
        title,
        image_url,
        target: parse_creative_action(creative),
        play_count: creative["resources"][0]["resourceExtInfo"]["playCount"].as_u64(),
    })
}

/// 首页 action 是字符串，雷达 action 是 clickAction 对象。
fn parse_creative_action(creative: &Value) -> Option<CreativeTarget> {
    let action = creative["action"]
        .as_str()
        .or_else(|| creative["action"]["clickAction"]["targetUrl"].as_str())
        .or_else(|| creative["resources"][0]["action"]["clickAction"]["targetUrl"].as_str())?;
    let path = action.strip_prefix("orpheus://").unwrap_or(action);
    let path = path.split('?').next().unwrap_or(path);
    Some(
        match path
            .strip_prefix("playlist/")
            .and_then(|id| id.parse::<u64>().ok())
        {
            Some(id) => CreativeTarget::Playlist(id),
            None => CreativeTarget::Link(action.to_owned()),
        },
    )
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

#[cfg(test)]
mod tests {
    use super::{CreativeTarget, PLAYLIST_BLOCK_CODE, parse_blocks};

    #[test]
    fn parses_playlist_block_creatives() {
        let blocks = parse_blocks(&serde_json::json!([
            {
                "blockCode": PLAYLIST_BLOCK_CODE,
                "uiElement": { "mainTitle": { "title": "" }, "subTitle": { "title": "推荐歌单" } },
                "creatives": [
                    {
                        "creativeType": "list",
                        "action": "orpheus://playlist/18413122437?autoplay=0&referLog=HOMEPAGE_BLOCK_PLAYLIST_RCMD",
                        "uiElement": {
                            "mainTitle": { "title": "无畏契约上海全球冠军赛" },
                            "image": { "imageUrl": "http://p3.music.126.net/a.jpg" }
                        },
                        "resources": [{ "resourceExtInfo": { "playCount": 2708957952_u64 } }]
                    },
                    {
                        "creativeType": "song",
                        "action": "orpheus://song/123",
                        "uiElement": { "mainTitle": { "title": "一首歌" } }
                    }
                ]
            }
        ]));
        assert_eq!(blocks.len(), 1);
        let block = &blocks[0];
        assert_eq!(block.code, PLAYLIST_BLOCK_CODE);
        assert_eq!(block.title, "推荐歌单");
        assert_eq!(block.creatives.len(), 2);
        let first = &block.creatives[0];
        assert_eq!(first.title, "无畏契约上海全球冠军赛");
        assert_eq!(
            first.image_url.as_deref(),
            Some("http://p3.music.126.net/a.jpg")
        );
        assert_eq!(first.target, Some(CreativeTarget::Playlist(18413122437)));
        assert_eq!(first.play_count, Some(2708957952));
        assert_eq!(
            block.creatives[1].target,
            Some(CreativeTarget::Link("orpheus://song/123".into()))
        );
    }

    #[test]
    fn tolerates_missing_fields_and_empty_blocks() {
        let blocks = parse_blocks(&serde_json::json!([
            { "blockCode": "HOMEPAGE_VOICEBOOK_RCMD", "creatives": [] },
            { "blockCode": "HOMEPAGE_BANNER" }
        ]));
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].title, "");
        assert!(blocks[0].creatives.is_empty());
        assert!(blocks[1].creatives.is_empty());
        assert!(parse_blocks(&serde_json::Value::Null).is_empty());
    }
}
