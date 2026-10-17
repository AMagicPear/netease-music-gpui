use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricLine {
    pub time: Duration,
    pub text: String,
    pub translation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SongComment {
    pub id: u64,
    pub nickname: String,
    pub avatar_url: String,
    pub content: String,
    /// API 原始时间戳，单位为毫秒。
    pub time: i64,
    pub liked_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SongComments {
    pub total: u64,
    /// 第一页里热评的条数：`comments` 前 `hot_count` 条是精选评论，
    /// UI 据此分出「精选评论」小节（最多取 5 条）；翻页请求里恒为 0。
    pub hot_count: usize,
    pub comments: Vec<SongComment>,
    pub more: bool,
}
