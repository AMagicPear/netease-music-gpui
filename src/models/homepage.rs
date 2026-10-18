//! 首页区块（`/homepage/block/page`）。
//!
//! 官方首页是一串区块：每块带自己的标题（「推荐歌单」「新歌新碟」……）和若干
//! 条目。条目（creative）形状统一：一张图 + 一个标题 + 一条 `orpheus://` 深链，
//! 有的还带播放量。页面按 `code` 挑区块来渲染，不要按数组下标取——顺序会变，
//! 区块也会随账号状态增减（游客 15 块、登录后 6 块起）。

use serde_json::Value;

/// 「推荐歌单」区块的 code：推荐页卡片行用它。
pub const PLAYLIST_BLOCK_CODE: &str = "HOMEPAGE_BLOCK_PLAYLIST_RCMD";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct HomepageBlock {
    pub code: String,
    /// 区块标题，取 `uiElement.subTitle.title`；没有标题的区块是空串。
    pub title: String,
    pub creatives: Vec<HomepageCreative>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct HomepageCreative {
    pub title: String,
    pub image_url: Option<String>,
    /// 深链解析出来的打开目标；认不出来的原样保留。
    pub target: Option<CreativeTarget>,
    pub play_count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CreativeTarget {
    /// 歌单：`orpheus://playlist/{id}`。
    Playlist(u64),
    /// 其他目标（播客、话题、视频……）。接入那个功能时再细分，先留着原串。
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

/// 解析一组 creative。首页区块和推荐页的新接口
/// （`/pc/personal/page/playlist/rcmd`、`/pc/customize/block/page`）用的是同一套形状。
pub(crate) fn parse_creatives(creatives: &Value) -> Vec<HomepageCreative> {
    creatives
        .as_array()
        .map(|creatives| creatives.iter().filter_map(parse_creative).collect())
        .unwrap_or_default()
}

fn parse_creative(creative: &Value) -> Option<HomepageCreative> {
    let title = text(&creative["uiElement"]["mainTitle"]["title"]);
    // 老首页区块把图放在 `image.imageUrl`，新接口（雷达歌单）用 `images[0].imageUrl`，
    // 两种都收，免得同一份 creative 因为位置不同被丢掉。
    let image_url = creative["uiElement"]["image"]["imageUrl"]
        .as_str()
        .or_else(|| creative["uiElement"]["images"][0]["imageUrl"].as_str())
        .map(str::to_owned);
    // 没图没标题的条目渲染不出来，直接跳过（比如「新歌」这种聚合卡片）。
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

/// 老首页区块的 `action` 是字符串；雷达等新接口给的是
/// `{ clickAction: { targetUrl } }` 对象，两种都认。
fn parse_creative_action(creative: &Value) -> Option<CreativeTarget> {
    let action = creative["action"]
        .as_str()
        .or_else(|| creative["action"]["clickAction"]["targetUrl"].as_str())
        .or_else(|| creative["resources"][0]["action"]["clickAction"]["targetUrl"].as_str())?;
    Some(parse_action(action))
}

/// `orpheus://playlist/123?autoplay=0&referLog=...` → 歌单 123。
fn parse_action(action: &str) -> CreativeTarget {
    let path = action.strip_prefix("orpheus://").unwrap_or(action);
    let path = path.split('?').next().unwrap_or(path);
    match path
        .strip_prefix("playlist/")
        .and_then(|id| id.parse::<u64>().ok())
    {
        Some(id) => CreativeTarget::Playlist(id),
        None => CreativeTarget::Link(action.to_owned()),
    }
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

#[cfg(test)]
mod tests {
    use super::{CreativeTarget, PLAYLIST_BLOCK_CODE, parse_blocks};

    /// 照抄 `/homepage/block/page` 的真实形状：区块标题在 `uiElement.subTitle`，
    /// 条目标题在 `uiElement.mainTitle`，播放量藏在 `resources[0].resourceExtInfo`。
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
        // 不是歌单的深链原样留着，别当成歌单 id 丢掉。
        assert_eq!(
            block.creatives[1].target,
            Some(CreativeTarget::Link("orpheus://song/123".into()))
        );
    }

    /// 缺字段、空区块都不能让整页解析失败。
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
