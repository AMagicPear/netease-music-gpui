//! 消息中心的四种消息（私信 / 评论 / @我 / 通知）统一成一份可渲染的条目。
//!
//! 四个接口的正文都藏在不同的嵌套位置，而且不少字段是「JSON 字符串里再包一层
//! JSON」（见 `src/api.rs` 里 `probe_msg_*` 探针测试记录的真实结构）。这里负责把
//! 这些形状各异的载荷拍平成 [`MessageItem`]：解析宽容，缺字段就留空，不因个别
//! 消息格式意外而让整个列表失败。

use serde_json::Value;

/// 一条消息在侧面板里渲染所需的全部信息。
///
/// 四个 tab 共用一个结构：`be_replied` / `source` / `unread` 只在部分 tab 有值，
/// 用 Option / 默认零值表达，行渲染按有无展示。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MessageItem {
    pub avatar_url: String,
    pub nickname: String,
    /// 消息正文（已剥掉嵌套 JSON 的最终文本）。
    pub content: String,
    /// 评论 tab：对方回复时所「回复的那条评论」的原文（通常是我发的）。
    pub be_replied: Option<String>,
    /// 评论 tab：评论所在的资源名（歌曲 / 歌单名）。仍会解析，UI 暂不显示，
    /// 留给以后做来源展示时用。
    pub source: Option<String>,
    /// 通知 tab：操作人做的事（如「赞了你的评论」），渲染成「昵称 + 动作」；其余 tab 为空。
    pub action: String,
    /// API 原始时间戳，单位为毫秒。
    pub time: i64,
    /// 私信 tab：这条会话的未读条数；其余 tab 恒为 0。
    pub unread: u32,
}

/// 私信会话列表（`/api/msg/private/users` 的 `msgs` 数组）。
///
/// 一条会话的双方由 `user.fromUserId` 指认：最后发言人是 `fromUser`，
/// 若那是我自己，会话对面的头像昵称就要取 `toUser`。
pub(crate) fn parse_private(msgs: &Value, uid: u64) -> Vec<MessageItem> {
    array_of(msgs)
        .iter()
        .map(|msg| {
            let last_sender = msg["user"]["fromUserId"].as_i64();
            let partner = if last_sender == Some(uid as i64) {
                &msg["toUser"]
            } else {
                &msg["fromUser"]
            };
            let (nickname, avatar_url) = user_of(partner);
            MessageItem {
                nickname,
                avatar_url,
                content: private_text(&msg["lastMsg"]),
                time: msg["lastMsgTime"].as_i64().unwrap_or(0),
                unread: msg["newMsgCount"].as_u64().unwrap_or(0) as u32,
                ..Default::default()
            }
        })
        .collect()
}

/// 评论消息（`/api/v1/user/comments/{uid}` 的 `comments` 数组）。
pub(crate) fn parse_comments(comments: &Value) -> Vec<MessageItem> {
    array_of(comments)
        .iter()
        .map(|comment| {
            let (nickname, avatar_url) = user_of(&comment["user"]);
            MessageItem {
                nickname,
                avatar_url,
                content: text_of(&comment["content"]).unwrap_or_default(),
                be_replied: non_empty(&comment["beRepliedContent"]),
                source: comment["resourceJson"]
                    .as_str()
                    .map(json_snippet)
                    .and_then(|resource| non_empty(&resource["name"])),
                time: comment["time"].as_i64().unwrap_or(0),
                ..Default::default()
            }
        })
        .collect()
}

/// @我（`/api/forwards/get` 的 `forwards` 数组）。
///
/// 实测至少两种形状（探针见各测试夹具）：MLOG 评论转发（@我的人在
/// `json.comment.user`，正文在 `comment.content`）和动态转发（人在外层
/// `json.user`，正文在再嵌一层的 `json.json` 字符串里）。按此顺序回退。
pub(crate) fn parse_forwards(forwards: &Value) -> Vec<MessageItem> {
    array_of(forwards)
        .iter()
        .map(|forward| {
            let payload = forward["json"].as_str().map(json_snippet).unwrap_or_default();
            let user = [
                &payload["comment"]["user"],
                &payload["user"],
                &payload["resource"]["userProfile"],
            ]
            .into_iter()
            .find(|user| user["nickname"].is_string())
            .unwrap_or(&Value::Null);
            let content = text_of(&payload["comment"]["content"])
                .or_else(|| forward_event_text(&payload))
                .or_else(|| text_of(&payload["resource"]["mlogBaseData"]["text"]))
                .unwrap_or_default();
            let (nickname, avatar_url) = user_of(user);
            MessageItem {
                nickname,
                avatar_url,
                content,
                time: forward["time"].as_i64().unwrap_or(0),
                ..Default::default()
            }
        })
        .collect()
}

/// 动态转发的正文：外层 `json.json` 字符串里还有一层 JSON，
/// 取其 `msg`，缺失时退到资源名（如「来自视频「…」」）。
fn forward_event_text(payload: &Value) -> Option<String> {
    let event = json_snippet(payload["json"].as_str()?);
    text_of(&event["msg"]).or_else(|| text_of(&event["resource"]["resourceName"]))
}

/// 系统通知（`/api/msg/notices` 的 `notices` 数组）。
///
/// `notice` 字符串按通知类型装着不同载荷；已知形态里 `user` 是操作人，
/// 评论类通知的正文在 `comment.content`。类型映射见 [`notice_action`]，
/// 未知类型退到空动作和空正文，只显示人名和时间。
pub(crate) fn parse_notices(notices: &Value) -> Vec<MessageItem> {
    array_of(notices)
        .iter()
        .map(|notice| {
            let payload = notice["notice"].as_str().map(json_snippet).unwrap_or_default();
            let (nickname, avatar_url) = user_of(&payload["user"]);
            MessageItem {
                nickname,
                avatar_url,
                action: notice_action(payload["type"].as_i64().unwrap_or(0)),
                // 评论类通知正文在 comment；收藏歌单类没有 content，
                // 退到歌单名，让第二行不至于空白。
                content: text_of(&payload["comment"]["content"])
                    .or_else(|| text_of(&payload["playlist"]["name"]))
                    .unwrap_or_default(),
                time: notice["time"].as_i64().unwrap_or(0),
                ..Default::default()
            }
        })
        .collect()
}

/// 通知类型的动作文案，渲染层拼成「昵称 + 动作」。
///
/// `type` 官方没有文档，以下均由探针实测：`6` 是评论被赞（载荷带 `comment`），
/// `2` 是歌单被收藏（载荷带 `playlist`）。其它类型遇到时跑 `api.rs` 里的
/// `probe_notices_raw` 探针对照真实载荷补表。
fn notice_action(kind: i64) -> String {
    match kind {
        2 => "收藏了你的歌单",
        6 => "赞了你的评论",
        _ => "",
    }
    .to_owned()
}

/// 数组字段：缺失或类型不对时当空数组，调用方无需判型。
fn array_of(value: &Value) -> &[Value] {
    static EMPTY: [Value; 0] = [];
    value.as_array().map(Vec::as_slice).unwrap_or(&EMPTY)
}

/// （昵称，头像 URL）：缺省为空串，头像为空时由渲染层回落到默认头像。
fn user_of(user: &Value) -> (String, String) {
    (
        text_of(&user["nickname"]).unwrap_or_default(),
        text_of(&user["avatarUrl"]).unwrap_or_default(),
    )
}

fn text_of(value: &Value) -> Option<String> {
    value.as_str().map(str::to_owned).filter(|s| !s.is_empty())
}

fn non_empty(value: &Value) -> Option<String> {
    text_of(value)
}

/// 接口里常见的「字段值是 JSON 字符串」；解析失败按 Null 处理。
fn json_snippet(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

/// 私信 `lastMsg` 的正文提取。
///
/// 它本身是 JSON 字符串，且 `msg` 字段还可能「再嵌一层 JSON 字符串」（官方账号
/// 群发消息），剥到纯文本为止；剥不开就按原样返回。`msg` 缺失时退到推送摘要。
fn private_text(last_msg: &Value) -> String {
    let Some(text) = last_msg.as_str().filter(|s| !s.is_empty()) else {
        return String::new();
    };
    let mut value = match serde_json::from_str::<Value>(text) {
        Ok(value) => value,
        // lastMsg 不是 JSON：按原文显示。
        Err(_) => return text.to_owned(),
    };
    for _ in 0..3 {
        let Some(nested_text) = value["msg"].as_str().map(str::to_owned) else {
            break;
        };
        match serde_json::from_str::<Value>(&nested_text) {
            // 内层仍是 JSON 且还带 msg：继续剥。
            Ok(nested) if nested["msg"].is_string() => value = nested,
            _ => return nested_text,
        }
    }
    value["msg"]
        .as_str()
        .or_else(|| value["pushMsg"].as_str())
        .or_else(|| value["generalMsg"]["inboxBriefContent"].as_str())
        .unwrap_or("")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const MY_UID: u64 = 497062445;

    #[test]
    fn private_picks_the_partner_not_the_last_sender() {
        // 真实响应（探针实测）：对方是最后发言人（fromUserId ≠ uid），直接取 fromUser。
        let msgs = json!([{
            "user": {"fromUserId": 6453460130u64, "toUserId": MY_UID},
            "fromUser": {"nickname": "云音乐小秘书", "avatarUrl": "http://a/secretary.jpg"},
            "toUser": {"nickname": "一只会魔法的梨", "avatarUrl": "http://a/me.jpg"},
            "lastMsg": "{\"msgId\":1,\"msg\":\"{\\\"msg\\\":\\\"你好呀\\\"}\"}",
            "lastMsgTime": 1791692547661i64,
            "newMsgCount": 1
        }]);
        let items = parse_private(&msgs, MY_UID);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].nickname, "云音乐小秘书");
        assert_eq!(items[0].content, "你好呀");
        assert_eq!(items[0].unread, 1);

        // 最后一条是我发的（fromUserId == uid）：对面是收信人 toUser。
        let flipped = json!([{
            "user": {"fromUserId": MY_UID, "toUserId": 6453460130u64},
            "fromUser": {"nickname": "一只会魔法的梨", "avatarUrl": ""},
            "toUser": {"nickname": "对方", "avatarUrl": ""}
        }]);
        assert_eq!(parse_private(&flipped, MY_UID)[0].nickname, "对方");
    }

    #[test]
    fn private_falls_back_to_push_summary_and_survives_bad_payloads() {
        let msgs = json!([
            {"fromUser": {"nickname": "官方", "avatarUrl": "http://a/x.jpg"},
             "lastMsg": "{\"pushMsg\":\"活动开始了\"}", "newMsgCount": 3},
            {"fromUser": {"nickname": "坏数据", "avatarUrl": "http://a/y.jpg"},
             "lastMsg": "不是JSON"},
            {"fromUser": {}}
        ]);
        let items = parse_private(&msgs, MY_UID);
        assert_eq!(items[0].content, "活动开始了");
        assert_eq!(items[0].unread, 3);
        assert_eq!(items[1].content, "不是JSON");
        assert_eq!(items[2].nickname, "");
        assert_eq!(items[2].content, "");
    }

    #[test]
    fn comments_extract_content_reply_context_and_source_name() {
        let comments = json!([{
            "user": {"nickname": "刘柏辛粉丝", "avatarUrl": "http://a/fan.jpg"},
            "content": "是的 传统声乐的依字行腔就是这么来的",
            "beRepliedContent": "我: 因为中文念词的时候其实也是有音高的",
            "resourceJson": "{\"resourceType\":4,\"id\":2000414968,\"name\":\"干嘛\",\"creator\":\"刘柏辛Lexie\"}",
            "time": 1774449756268i64
        }]);
        let items = parse_comments(&comments);
        assert_eq!(items[0].content, "是的 传统声乐的依字行腔就是这么来的");
        assert_eq!(
            items[0].be_replied.as_deref(),
            Some("我: 因为中文念词的时候其实也是有音高的")
        );
        assert_eq!(items[0].source.as_deref(), Some("干嘛"));
        assert_eq!(items[0].time, 1774449756268);
    }

    #[test]
    fn forwards_prefer_the_mention_comment_and_fall_back_to_resource() {
        let forwards = json!([{
            "json": "{\"resource\":{\"mlogBaseData\":{\"text\":\"哈哈\"},\"userProfile\":{\"nickname\":\"MLOG作者\",\"avatarUrl\":\"http://a/m.jpg\"}},\"comment\":{\"user\":{\"nickname\":\"@我的朋友\",\"avatarUrl\":\"http://a/f.jpg\"},\"content\":\"@一只会魔法的梨\"}}",
            "time": 1680556869288i64
        }, {
            // 没有评论载荷的转发：退到资源作者与资源文本。
            "json": "{\"resource\":{\"mlogBaseData\":{\"text\":\"纯MLOG\"},\"userProfile\":{\"nickname\":\"作者B\",\"avatarUrl\":\"http://a/b.jpg\"}}}",
            "time": 1
        }, {
            // 动态转发（探针实测缩形）：人在外层 user，正文在再嵌一层的 json 字符串。
            "json": "{\"user\":{\"nickname\":\"捣药兔要有长进\",\"avatarUrl\":\"http://a/r.jpg\"},\"json\":\"{\\\"msg\\\":\\\"\\\",\\\"resource\\\":{\\\"resourceName\\\":\\\"来自视频「圈出那个人」\\\"}}\"}",
            "time": 2
        }]);
        let items = parse_forwards(&forwards);
        assert_eq!(items[0].nickname, "@我的朋友");
        assert_eq!(items[0].content, "@一只会魔法的梨");
        assert_eq!(items[1].nickname, "作者B");
        assert_eq!(items[1].content, "纯MLOG");
        assert_eq!(items[2].nickname, "捣药兔要有长进");
        assert_eq!(items[2].content, "来自视频「圈出那个人」");
    }

    #[test]
    fn notices_read_the_actor_action_and_comment_payload() {
        let notices = json!([{
            "notice": "{\"type\":6,\"user\":{\"nickname\":\"--treestrees--\",\"avatarUrl\":\"http://a/t.jpg\"},\"comment\":{\"content\":\"整首的编排都很有感觉\"}}",
            "time": 1791603957364i64
        }, {
            // 探针实测（2026-05-21 的收藏通知）：type=2，载荷带 playlist.name。
            "notice": "{\"type\":2,\"user\":{\"nickname\":\"Sylvia轩子\"},\"playlist\":{\"name\":\"欧美半商业有词流行电子\"}}",
            "time": 1779367381832i64
        }, {
            // 未知通知类型：有人有时间，动作退到空。
            "notice": "{\"user\":{\"nickname\":\"系统\"}}",
            "time": 1
        }]);
        let items = parse_notices(&notices);
        assert_eq!(items[0].nickname, "--treestrees--");
        assert_eq!(items[0].action, "赞了你的评论");
        assert_eq!(items[0].content, "整首的编排都很有感觉");
        assert_eq!(items[1].nickname, "Sylvia轩子");
        assert_eq!(items[1].action, "收藏了你的歌单");
        assert_eq!(items[1].content, "欧美半商业有词流行电子");
        assert_eq!(items[1].source, None);
        assert_eq!(items[2].nickname, "系统");
        assert_eq!(items[2].action, "");
        assert_eq!(items[2].content, "");
        assert_eq!(items[2].source, None);
    }

    #[test]
    fn missing_or_mistyped_arrays_yield_no_items() {
        assert!(parse_private(&json!(null), MY_UID).is_empty());
        assert!(parse_comments(&json!({})).is_empty());
        assert!(parse_forwards(&json!("abc")).is_empty());
        assert!(parse_notices(&json!([1, 2])).len() == 2);
    }
}
