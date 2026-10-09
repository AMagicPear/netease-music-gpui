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
    pub comments: Vec<SongComment>,
    pub more: bool,
}
