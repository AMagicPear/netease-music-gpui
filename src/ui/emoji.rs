//! 评论里的网易云表情：`[大笑]`、`[强]` 这类方括号标记，服务端存的就是「表情名」。
//!
//! 官方那套图本身是 emoji 的渲染结果，所以这里不插图片：解析出名字后直接换成对应的
//! Unicode 字符，交给系统字体画彩色字形。macOS 的文本后端认 AppleColorEmoji
//! （`is_emoji` 在 gpui-pre-macos 里会走 RGBA bitmap context），字形、基线、行高、
//! 选中和复制都由文本系统统一处理，比在文本流里内嵌图片省掉一整套排版逻辑。
//!
//! 表里的编号是资源编号，仍然对应 `assets/images/emoji/emoji_<id>.png`（63×63 的
//! `@3x` 图）。它们现在不参与渲染，只作为「名字 ↔ 形象」的对照与将来回退用；
//! 映射关系是用这些图逐张比对 Apple emoji 字形定下来的，不是靠名字猜的。

// 图片路径与编号目前只有测试和取图脚本在用；接入图片回退时删掉这行。
#![allow(dead_code)]

use std::borrow::Cow;

/// 表情表：(中文名, 资源编号, Unicode 字符)。
///
/// 名字唯一，顺序不参与匹配；`18` 这种数字名也是合法表情。
const EMOJIS: &[(&str, u32, &str)] = &[
    ("大笑", 86, "😃"),
    ("可爱", 85, "😊"),
    ("憨笑", 359, "☺️"),
    ("色", 95, "😍"),
    ("亲亲", 363, "😘"),
    ("惊恐", 96, "😱"),
    ("流泪", 356, "😭"),
    ("亲", 362, "😚"),
    ("呆", 352, "😳"),
    ("哀伤", 342, "😔"),
    ("呲牙", 343, "😁"),
    ("吐舌", 348, "😝"),
    ("撇嘴", 353, "😒"),
    ("怒", 361, "😡"),
    ("奸笑", 341, "😏"),
    ("汗", 97, "😓"),
    ("痛苦", 346, "😖"),
    ("惶恐", 354, "😰"),
    ("生病", 350, "😨"),
    ("口罩", 351, "😷"),
    ("大哭", 357, "😂"),
    ("晕", 355, "😵"),
    ("发怒", 115, "👿"),
    ("开心", 360, "😄"),
    ("鬼脸", 94, "😜"),
    ("皱眉", 87, "😞"),
    // 名字写成「流感」，图其实是皱眉 + 单滴泪，即 😢；以官方图为准，不按名字猜。
    ("流感", 358, "😢"),
    ("爱心", 33, "❤️"),
    ("心碎", 34, "💔"),
    ("钟情", 303, "💘"),
    ("星星", 309, "⭐"),
    ("生气", 314, "💢"),
    ("便便", 89, "💩"),
    ("强", 13, "👍"),
    ("弱", 372, "👎"),
    ("拜", 14, "🙏"),
    ("牵手", 379, "👫"),
    ("跳舞", 380, "👯"),
    ("禁止", 374, "🙅"),
    ("这边", 262, "👉"),
    ("爱意", 106, "💏"),
    ("示爱", 376, "💑"),
    ("嘴唇", 367, "👄"),
    ("狗", 81, "🐶"),
    ("猫", 78, "🐱"),
    ("猪", 100, "🐷"),
    ("兔子", 459, "🐰"),
    ("小鸡", 450, "🐤"),
    ("公鸡", 461, "🐔"),
    ("幽灵", 116, "👻"),
    ("圣诞", 411, "🎅"),
    ("外星", 101, "👽"),
    ("钻石", 52, "💎"),
    ("礼物", 107, "🎁"),
    ("男孩", 0, "👦"),
    ("女孩", 1, "👧"),
    ("蛋糕", 337, "🎂"),
    ("18", 186, "🔞"),
    ("圈", 312, "⭕"),
    ("叉", 313, "❌"),
];

/// 方括号里最多允许这么多个字节：最长的名字是两个汉字（6 字节），
/// 留一点余量，同时避免把 `[这是正常的方括号说明]` 整段拿去查表。
const MAX_NAME_BYTES: usize = 16;

/// 评论正文切出来的一段：普通文字，或一个表情。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Segment<'a> {
    Text(&'a str),
    Emoji { name: &'a str, ch: &'static str },
}

/// 表情名 → (Unicode 字符, 资源编号)。
pub fn emoji(name: &str) -> Option<(&'static str, u32)> {
    EMOJIS
        .iter()
        .find_map(|&(known, id, ch)| (known == name).then_some((ch, id)))
}

/// 表情名 → 图片编号；只有需要回退到图片渲染时才有用。
pub fn emoji_id(name: &str) -> Option<u32> {
    emoji(name).map(|(_, id)| id)
}

/// 表情图在 `assets/` 下的相对路径。
///
/// 这些图片只存在于本地（抓取来核对映射用，未随仓库提交），渲染不依赖它们；
/// 保留路径拼装是为了本地对照与将来回退到图片时能直接用。
pub fn asset_path(id: u32) -> String {
    format!("images/emoji/emoji_{id}.png")
}

/// 把一条评论文本切成文字 / 表情片段。未登记的方括号原样留在文字里，
/// 不会吞掉 `[已删除]` 之类的普通内容。
pub fn segments(text: &str) -> Vec<Segment<'_>> {
    let bytes = text.as_bytes();
    let mut segments = Vec::new();
    let mut text_start = 0;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'['
            && let Some(offset) = text[index + 1..].find(']')
        {
            let close = index + 1 + offset;
            let name = &text[index + 1..close];
            if name.len() <= MAX_NAME_BYTES
                && let Some((ch, _)) = emoji(name)
            {
                if text_start < index {
                    segments.push(Segment::Text(&text[text_start..index]));
                }
                segments.push(Segment::Emoji { name, ch });
                index = close + 1;
                text_start = index;
                continue;
            }
        }
        // 按 UTF-8 字符前进，避免切在多字节字符中间。
        index += text[index..]
            .chars()
            .next()
            .map_or(1, char::len_utf8);
    }
    if text_start < text.len() {
        segments.push(Segment::Text(&text[text_start..]));
    }
    segments
}

/// 把评论正文里的 `[表情名]` 换成 Unicode 字符，交给文字元素直接渲染。
///
/// 没有可识别的表情时返回借用的原字符串，正常评论不额外分配。
pub fn expand(text: &str) -> Cow<'_, str> {
    if !text.contains('[') {
        return Cow::Borrowed(text);
    }
    let parts = segments(text);
    if !parts.iter().any(|part| matches!(part, Segment::Emoji { .. })) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + 8);
    for part in parts {
        match part {
            Segment::Text(run) => out.push_str(run),
            Segment::Emoji { ch, .. } => out.push_str(ch),
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::{Segment, asset_path, emoji, emoji_id, expand, segments};

    #[test]
    fn known_emoji_is_its_own_segment() {
        assert_eq!(
            segments("笑死[大笑]"),
            [
                Segment::Text("笑死"),
                Segment::Emoji {
                    name: "大笑",
                    ch: "😃"
                }
            ]
        );
    }

    #[test]
    fn adjacent_and_edge_emojis_keep_every_piece() {
        assert_eq!(
            segments("[强][弱]"),
            [
                Segment::Emoji {
                    name: "强",
                    ch: "👍"
                },
                Segment::Emoji {
                    name: "弱",
                    ch: "👎"
                }
            ]
        );
        assert_eq!(
            segments("开头[狗]中间[猫]"),
            [
                Segment::Text("开头"),
                Segment::Emoji {
                    name: "狗",
                    ch: "🐶"
                },
                Segment::Text("中间"),
                Segment::Emoji {
                    name: "猫",
                    ch: "🐱"
                }
            ]
        );
        assert!(segments("没有表情").len() == 1);
        assert!(segments("").is_empty());
    }

    #[test]
    fn unknown_or_unclosed_brackets_stay_text() {
        assert_eq!(segments("[不存在]"), [Segment::Text("[不存在]")]);
        assert_eq!(segments("[大笑"), [Segment::Text("[大笑")]);
        assert_eq!(
            segments("[大笑]"),
            [Segment::Emoji {
                name: "大笑",
                ch: "😃"
            }]
        );
        assert_eq!(
            segments("[a]b[大笑]"),
            [
                Segment::Text("[a]b"),
                Segment::Emoji {
                    name: "大笑",
                    ch: "😃"
                }
            ]
        );
    }

    #[test]
    fn numeric_name_is_a_valid_emoji() {
        assert_eq!(emoji_id("18"), Some(186));
        assert_eq!(
            segments("[18]"),
            [Segment::Emoji {
                name: "18",
                ch: "🔞"
            }]
        );
    }

    #[test]
    fn expand_replaces_only_known_emoji() {
        assert_eq!(expand("今天真的顶[强]"), "今天真的顶👍");
        assert_eq!(expand("[强][弱]"), "👍👎");
        // 未登记的方括号、未闭合的方括号都原样保留。
        assert_eq!(expand("[已删除] 这条"), "[已删除] 这条");
        assert_eq!(expand("[大笑"), "[大笑");
    }

    #[test]
    fn expand_borrows_when_nothing_changes() {
        assert!(matches!(expand("普通评论"), std::borrow::Cow::Borrowed(_)));
        assert!(matches!(
            expand("[不存在]"),
            std::borrow::Cow::Borrowed(_)
        ));
        assert!(matches!(expand("[强]"), std::borrow::Cow::Owned(_)));
    }

    #[test]
    fn table_has_no_duplicate_names_or_ids() {
        let names: std::collections::HashSet<_> = super::EMOJIS.iter().map(|e| e.0).collect();
        let ids: std::collections::HashSet<_> = super::EMOJIS.iter().map(|e| e.1).collect();
        assert_eq!(names.len(), super::EMOJIS.len());
        assert_eq!(ids.len(), super::EMOJIS.len());
        // 每个字符都非空，避免表里出现空字符串导致 `[x]` 被静默吞掉。
        for (name, _, ch) in super::EMOJIS {
            assert!(!ch.is_empty(), "{name} 没有对应字符");
            assert!(emoji(name).is_some());
        }
    }

    #[test]
    fn asset_path_matches_packaged_file_name() {
        assert_eq!(asset_path(13), "images/emoji/emoji_13.png");
        assert_eq!(asset_path(0), "images/emoji/emoji_0.png");
    }
}
