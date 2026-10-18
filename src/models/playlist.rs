use serde::{Deserialize, Serialize};

use super::user::UserProfile;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: u64,
    pub name: String,
    pub cover_img_url: Option<String>,
    pub creator: UserProfile,
    /// 创建时间（毫秒）。`playlist/detail` 会给，搜索结果里的歌单项不给，
    /// 所以是可选的：缺了就是「创建时间未知」，不该替它编一个假日期出来。
    pub create_time: Option<i64>,
    pub play_count: u64,
    pub track_count: usize,
    /// 收藏数。同一个指标接口给了两个名字：`playlist/detail` 叫 `subscribedCount`，
    /// 搜索结果叫 `bookCount`。用 alias 收进同一个字段，调用方只认「收藏数」这一件事。
    #[serde(alias = "bookCount")]
    pub subscribed_count: u64,
    pub comment_count: Option<u64>,
    /// 歌单简介；API 返回的多行文本原样保存，压成一行是展示层的事。
    #[serde(default)]
    pub description: Option<String>,
    /// 歌单标签；没有标签时 API 给的是 `null` 而不是 `[]`，所以保留 Option。
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    pub special_type: u32,
    #[serde(default)]
    pub track_ids: Vec<TrackId>,
}

impl Playlist {
    /// 展示用的简介：去掉首尾空白，纯空白和 `null` 都算「没有简介」。
    pub fn description(&self) -> Option<&str> {
        self.description
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
    }

    /// 展示用的标签：`null` 和空数组统一成空切片，调用方只看「有没有」。
    pub fn tags(&self) -> &[String] {
        self.tags.as_deref().unwrap_or_default()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TrackId {
    pub id: u64,
}

/// 「推荐歌单」条目（`/recommend/resource`）：只带推荐页卡片需要的字段。
///
/// 不复用 [`Playlist`]：这个接口给的是摘要形态（`picUrl`、小写的 `playcount`，
/// 没有创建者、标签、trackIds），硬塞进 `Playlist` 只会得到一堆空字段；
/// 点开歌单时按 id 走 `playlist/detail` 拿完整数据，这里不必冒充详情。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendedPlaylist {
    pub id: u64,
    pub name: String,
    /// 封面地址；接口可能给 `picUrl` 也可能给 `coverImgUrl`，都收进这一个字段。
    #[serde(alias = "coverImgUrl")]
    pub pic_url: Option<String>,
    /// 播放量。这个接口用的是小写的 `playcount`，`playCount` 写法一并兼容；
    /// 整个缺省时按 0 算——卡片显示「0」也比解析失败、整页没内容好。
    #[serde(default, alias = "playcount")]
    pub play_count: u64,
}

#[cfg(test)]
mod tests {
    use super::{Playlist, RecommendedPlaylist};

    /// 照抄 `/recommend/resource` 的真实响应：封面叫 `picUrl`、播放量叫小写的
    /// `playcount`，且没有创建者 / 标签 / trackIds。字段名写错（比如按 `Playlist`
    /// 的 `coverImgUrl` / `playCount` 解析）会在这里静默丢成默认值。
    #[test]
    fn recommended_playlist_parses_recommend_resource_shape() {
        let playlist: RecommendedPlaylist = serde_json::from_value(serde_json::json!({
            "id": 1476194476_u64,
            "type": 1,
            "name": "恢复能量歌单",
            "copywriter": "恢复能量歌单",
            "picUrl": "http://p2.music.126.net/cover.jpg?imageView&thumbnail=252x0",
            "canDislike": true,
            "playcount": 5182
        }))
        .expect("夹具应当能解析");
        assert_eq!(playlist.id, 1476194476);
        assert_eq!(playlist.name, "恢复能量歌单");
        assert_eq!(
            playlist.pic_url.as_deref(),
            Some("http://p2.music.126.net/cover.jpg?imageView&thumbnail=252x0")
        );
        assert_eq!(playlist.play_count, 5182);
    }

    /// 封面与播放量都允许缺省：卡片有兜底封面，播放量缺了就当 0，
    /// 不该让整个列表解析失败。
    #[test]
    fn recommended_playlist_falls_back_on_missing_cover_and_count() {
        let playlist: RecommendedPlaylist = serde_json::from_value(serde_json::json!({
            "id": 1, "name": "歌单"
        }))
        .expect("夹具应当能解析");
        assert_eq!(playlist.pic_url, None);
        assert_eq!(playlist.play_count, 0);
    }

    /// 照抄搜索歌单（cloudsearch type=1000）的真实响应：没有 `createTime` / `trackIds` /
    /// `tags`，收藏数叫 `bookCount`，创建者头像是 `null`。任何一项变回必填都会在这里解析失败。
    #[test]
    fn search_summary_parses_and_falls_back_on_missing_fields() {
        let playlist: Playlist = serde_json::from_value(serde_json::json!({
            "id": 6650447703_u64,
            "name": "哈哈哈哈笑声",
            "coverImgUrl": "http://p1.music.126.net/cover.jpg",
            "creator": {
                "nickname": "aaaaaaaaaaadg", "userId": 258173789, "userType": 0,
                "avatarUrl": null, "authStatus": 0
            },
            "subscribed": false,
            "trackCount": 1,
            "userId": 258173789,
            "playCount": 31591,
            "bookCount": 45,
            "specialType": 0,
            "officialTags": null,
            "action": null,
            "actionType": null,
            "recommendText": null,
            "score": null,
            "officialPlaylistTitle": null,
            "playlistType": "UGC",
            "description": null,
            "highQuality": false
        }))
        .unwrap();
        assert_eq!(playlist.name, "哈哈哈哈笑声");
        assert_eq!(playlist.track_count, 1);
        assert_eq!(playlist.play_count, 31591);
        // bookCount 就是详情里的 subscribedCount：同一个收藏数的两种叫法。
        assert_eq!(playlist.subscribed_count, 45);
        assert_eq!(playlist.creator.nickname, "aaaaaaaaaaadg");
        // 摘要没有的字段留空，展示层按「未知 / 没有」处理，不编数据。
        assert_eq!(playlist.create_time, None);
        assert!(playlist.track_ids.is_empty());
        assert_eq!(playlist.comment_count, None);
        assert_eq!(playlist.creator.avatar_url, "");
    }

    #[test]
    fn description_and_tags_ignore_missing_null_and_blank() {
        // 歌单列表接口返回的简要信息里可能整个不带这两个字段。
        let missing: Playlist = serde_json::from_value(serde_json::json!({
            "id": 1, "name": "歌单", "creator": {"userId": 1, "nickname": "我", "avatarUrl": ""},
            "createTime": 0, "playCount": 0, "trackCount": 0, "subscribedCount": 0,
            "specialType": 0
        }))
        .unwrap();
        assert_eq!(missing.description(), None);
        assert!(missing.tags().is_empty());
        assert_eq!(missing.create_time, Some(0));

        let nulled: Playlist = serde_json::from_value(serde_json::json!({
            "id": 1, "name": "歌单", "creator": {"userId": 1, "nickname": "我", "avatarUrl": ""},
            "createTime": 0, "playCount": 0, "trackCount": 0, "subscribedCount": 0,
            "specialType": 0, "description": null, "tags": null
        }))
        .unwrap();
        assert_eq!(nulled.description(), None);
        assert!(nulled.tags().is_empty());

        // 空白简介和空数组和「没有」等价，界面上一律不显示。
        let blank: Playlist = serde_json::from_value(serde_json::json!({
            "id": 1, "name": "歌单", "creator": {"userId": 1, "nickname": "我", "avatarUrl": ""},
            "createTime": 0, "playCount": 0, "trackCount": 0, "subscribedCount": 0,
            "specialType": 0, "description": "   ", "tags": []
        }))
        .unwrap();
        assert_eq!(blank.description(), None);
        assert!(blank.tags().is_empty());

        let full: Playlist = serde_json::from_value(serde_json::json!({
            "id": 1, "name": "歌单", "creator": {"userId": 1, "nickname": "我", "avatarUrl": ""},
            "createTime": 0, "playCount": 0, "trackCount": 0, "subscribedCount": 0,
            "specialType": 0, "description": " 华语精选\n ", "tags": ["华语", "流行"]
        }))
        .unwrap();
        // 原始文本保留换行，取用时才 trim——简介本身可能就是多行的。
        assert_eq!(full.description.as_deref(), Some(" 华语精选\n "));
        assert_eq!(full.description(), Some("华语精选"));
        assert_eq!(full.tags(), ["华语", "流行"]);
    }
}
