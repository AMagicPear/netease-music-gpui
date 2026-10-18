//! 评论的数据模型与它的 JSON 解析。
//!
//! 歌曲（`/api/v1/resource/comments/R_SO_4_{id}`）和歌单（`A_PL_0_{id}`）返回同一种
//! 响应形状，所以两者共用模型与解析器：UI 侧只需认资源类型，不必知道命中的是哪个接口。

use serde_json::Value;
use std::collections::HashSet;

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

/// 解析一页评论。`offset` 是这一页普通评论的起始位置：热评只合并在第一页
/// （`offset == 0`），否则接口也会把热评塞回来，翻页时整页都是重复数据。
pub(crate) fn parse_page(body: Value, offset: usize) -> Result<SongComments, String> {
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
        serde_json::from_value(body).map_err(|error| format!("评论格式无效：{error}"))?;
    let mut seen = HashSet::new();
    let hot = if offset == 0 {
        page.hot_comments.unwrap_or_default()
    } else {
        Vec::new()
    };
    // 热评排在最前，去重后剩下多少条就是精选小节的行数；
    // 热评内部也可能重复，所以数的是去重后真正入列的条数。
    let mut hot_count = 0usize;
    let comments = hot
        .into_iter()
        .map(|comment| (comment, true))
        .chain(page.comments.into_iter().map(|comment| (comment, false)))
        .filter(|(comment, _)| seen.insert(comment.comment_id))
        .map(|(comment, is_hot)| {
            if is_hot {
                hot_count += 1;
            }
            SongComment {
                id: comment.comment_id,
                nickname: comment.user.nickname,
                avatar_url: comment.user.avatar_url,
                content: comment.content,
                time: comment.time,
                liked_count: comment.liked_count,
            }
        })
        .collect();
    Ok(SongComments {
        total: page.total,
        hot_count,
        comments,
        more: page.more,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_page;

    #[test]
    fn hot_comments_lead_only_the_first_page_and_duplicates_are_dropped() {
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
        let first = parse_page(body.clone(), 0).unwrap();
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
        let next = parse_page(body, 20).unwrap();
        assert_eq!(
            next.comments.iter().map(|item| item.id).collect::<Vec<_>>(),
            [2, 3]
        );
        assert_eq!(next.comments[0].content, "重复");
        let empty = parse_page(
            serde_json::json!({"total": 0, "comments": [], "more": false}),
            0,
        )
        .unwrap();
        assert!(empty.comments.is_empty());
        assert!(!empty.more);
        assert!(parse_page(serde_json::json!({"comments": []}), 0).is_err());
    }
}
